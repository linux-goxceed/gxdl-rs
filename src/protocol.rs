use std::{
    io::{self, Write},
    thread,
    time::{Duration, Instant},
};

use crate::{
    AppResult,
    loader::BootImage,
    serial::{Transport, pulse_reset},
};

const STAGE1_TERMINATOR: &[u8] = b"boot";
const GX_KEY: [u8; 4] = [0x12, 0x34, 0x56, 0x78];

pub struct Session<T: Transport> {
    io: T,
    pending: Vec<u8>,
    verbose: bool,
}

impl<T: Transport> Session<T> {
    pub fn new(io: T, verbose: bool) -> Self {
        Self {
            io,
            pending: Vec::new(),
            verbose,
        }
    }

    pub fn boot(
        &mut self,
        image: &BootImage,
        reset_dtr: bool,
        reset_rts: bool,
    ) -> AppResult<Vec<u8>> {
        println!(
            "[+] Loaded boot loader: {} ({} bytes)",
            image.name,
            image.len()
        );
        println!(
            "    Version: 0x{:04X}, Chip: 0x{:04X}, Baud: {}",
            image.version, image.chip, image.baud
        );

        pulse_reset(&mut self.io, reset_dtr, reset_rts)?;
        self.io
            .discard_buffers()
            .map_err(|error| format!("failed to flush serial buffers: {error}"))?;
        self.pending.clear();

        self.log("Waiting for device handshake; power-cycle or reset the device now");
        let handshake = self.wait_for_handshake(Duration::from_secs(30))?;
        self.log(&format!("Handshake detected: {}", hex(&handshake)));

        self.log("Sending Stage 1");
        let header = image.stage1_header();
        let mut stage1 = Vec::with_capacity(header.len() + image.stage1_payload().len() + 4);
        stage1.extend_from_slice(&header);
        stage1.extend_from_slice(image.stage1_payload());
        stage1.extend_from_slice(STAGE1_TERMINATOR);
        self.write(&stage1)?;
        self.flush()?;
        self.log(&format!("Stage 1 complete: {} bytes", stage1.len()));

        self.wait_for_runget(Duration::from_secs(15))?;
        thread::sleep(Duration::from_millis(50));

        let (metadata, content) = image.stage2();
        self.log(&format!("Sending Stage 2 metadata: {}", hex(&metadata)));
        self.write(&metadata)?;
        self.send_chunks(&content, 2048, "Loader")?;
        self.flush()?;

        println!("[+] Boot sequence transferred; reading device output:");
        let output = self.read_boot_output(Duration::from_secs(15))?;
        println!("[+] Upload successful");
        Ok(output)
    }

    pub fn wait_for_prompt(&mut self, timeout: Duration) -> AppResult<()> {
        let deadline = Instant::now() + timeout;
        let poke_at = Instant::now() + Duration::from_millis(200);
        let mut poked = false;

        loop {
            if let Some(index) = find_bytes(&self.pending, b"boot>") {
                self.pending.drain(..index + b"boot>".len());
                while self
                    .pending
                    .first()
                    .is_some_and(|byte| byte.is_ascii_whitespace())
                {
                    self.pending.remove(0);
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("timeout waiting for boot> prompt".into());
            }
            if !poked && Instant::now() >= poke_at {
                self.write(b"\n")?;
                self.flush()?;
                poked = true;
            }
            self.read_more(false)?;
        }
    }

    pub fn text_command(&mut self, command: &str, timeout: Duration) -> AppResult<String> {
        self.wait_for_prompt(Duration::from_secs(2))?;
        self.log(&format!("Sending command: {command}"));
        self.write(command.as_bytes())?;
        self.write(b"\n")?;
        self.flush()?;

        let response = self.read_until(b"boot>", timeout)?;
        Ok(clean_text_response(command, &response))
    }

    pub fn binary_read(&mut self, command: &str, size: usize) -> AppResult<(Vec<u8>, Option<u32>)> {
        if size == 0 {
            return Err("binary read length must be greater than zero".into());
        }
        self.wait_for_prompt(Duration::from_secs(2))?;
        self.send_echoed_command(command, Duration::from_secs(5))?;
        self.read_until(b"~sta~", Duration::from_secs(10))?;

        println!("[*] Receiving {size} bytes");
        let started = Instant::now();
        let timeout =
            Duration::from_secs(120).max(Duration::from_secs_f64(size as f64 / 10_000.0 + 60.0));
        let data = self.read_exact_with_progress(size, timeout, started)?;

        let crc = match self.read_until(b"~crc~", Duration::from_secs(5)) {
            Ok(_) => {
                let bytes = self.read_exact_pending(4, Duration::from_secs(2), "device CRC")?;
                Some(u32::from_le_bytes(
                    bytes.try_into().expect("four CRC bytes"),
                ))
            }
            Err(error) => {
                eprintln!("[!] Warning: {error}");
                None
            }
        };

        if self.read_until(b"~fin~", Duration::from_secs(2)).is_err() {
            self.log("No ~fin~ marker received after binary read");
        }
        if let Some(crc) = crc {
            self.log(&format!("Device-reported CRC: 0x{crc:08X} (not verified)"));
        }
        Ok((data, crc))
    }

    pub fn binary_write(
        &mut self,
        command: &str,
        data: &[u8],
        completion_timeout: Duration,
        wait_for_reboot: bool,
    ) -> AppResult<String> {
        if data.is_empty() {
            return Err("refusing to send an empty binary image".into());
        }
        self.wait_for_prompt(Duration::from_secs(2))?;
        self.send_echoed_command(command, Duration::from_secs(5))?;
        self.read_until(b"~sta~", Duration::from_secs(10))?;

        let started = Instant::now();
        print_progress(0, data.len(), started, "Send");
        for (index, chunk) in data.chunks(1024).enumerate() {
            self.write(chunk)?;
            self.flush()?;
            let sent = ((index + 1) * 1024).min(data.len());
            print_progress(sent, data.len(), started, "Send");
        }
        self.read_until(b"~crc~", Duration::from_secs(10))?;

        let checksum = gx_checksum(data);
        self.write(&checksum.to_be_bytes())?;
        self.flush()?;
        self.log(&format!("Sent GX checksum 0x{checksum:08X}"));

        let response = self.read_completion(completion_timeout, wait_for_reboot)?;
        let text = latin1(&response);
        let lower = text.to_ascii_lowercase();
        if lower.contains("err") && lower.contains("crc") {
            return Err(format!("device rejected checksum: {}", text.trim()));
        }
        Ok(text)
    }

    fn read_completion(&mut self, timeout: Duration, wait_for_reboot: bool) -> AppResult<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        let mut response = Vec::new();
        let mut saw_fin = false;

        loop {
            if !self.pending.is_empty() {
                response.append(&mut self.pending);
                saw_fin |= find_bytes(&response, b"~fin~").is_some();
                let prompt = find_bytes(&response, b"boot>").is_some();
                let partition = find_bytes(&response, b"Partition Version").is_some();
                let error = find_ascii_case_insensitive(&response, b"err");
                if error || prompt || partition || (saw_fin && !wait_for_reboot) {
                    return Ok(response);
                }
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timeout waiting for device completion; last response: {}",
                    latin1(&response).trim()
                ));
            }
            self.read_more(false)?;
        }
    }

    fn send_echoed_command(&mut self, command: &str, timeout: Duration) -> AppResult<()> {
        self.log(&format!("Sending command: {command}"));
        self.write(command.as_bytes())?;
        self.write(b"\n")?;
        self.flush()?;
        self.read_until(command.as_bytes(), timeout)
            .map(|_| ())
            .map_err(|_| format!("device did not echo command '{command}'"))
    }

    fn wait_for_handshake(&mut self, timeout: Duration) -> AppResult<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some((start, end)) = find_handshake(&self.pending) {
                let handshake = self.pending[start..end].to_vec();
                self.pending.clear();
                // IPL output can continue briefly after the final 0x58.
                thread::sleep(Duration::from_millis(5));
                self.io
                    .discard_buffers()
                    .map_err(|error| format!("failed to flush IPL noise: {error}"))?;
                return Ok(handshake);
            }
            if self.pending.len() > 32 {
                let remove = self.pending.len() - 32;
                self.pending.drain(..remove);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timeout waiting for handshake; last bytes: {}",
                    hex(&self.pending)
                ));
            }
            self.read_more(true)?;
        }
    }

    fn wait_for_runget(&mut self, timeout: Duration) -> AppResult<()> {
        let deadline = Instant::now() + timeout;
        let mut got_run_at = None;
        loop {
            if find_case_insensitive(&self.pending, b"RUNGET").is_some() {
                println!("[*] Received RUN");
                println!("[*] Received GET");
                self.pending.clear();
                return Ok(());
            }
            if find_tolerant_runget(&self.pending, 4).is_some() {
                println!("[*] Detected tolerant RUNGET");
                self.pending.clear();
                return Ok(());
            }
            if find_ordered_runget(&self.pending, 40).is_some() {
                println!("[*] Detected RUNGET through IPL noise");
                self.pending.clear();
                return Ok(());
            }
            if find_token(&self.pending, b"RUN").is_some() && got_run_at.is_none() {
                println!("[*] Received RUN");
                got_run_at = Some(Instant::now());
            }
            if got_run_at.is_some() && find_token(&self.pending, b"GET").is_some() {
                println!("[*] Received GET");
                self.pending.clear();
                return Ok(());
            }
            if got_run_at.is_some_and(|time| time.elapsed() >= Duration::from_secs(1)) {
                self.log("Proceeding after RUN without an explicit GET");
                self.pending.clear();
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timeout waiting for RUNGET; received: {}",
                    latin1(&self.pending)
                ));
            }
            self.read_more(true)?;
        }
    }

    fn read_boot_output(&mut self, timeout: Duration) -> AppResult<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let mut last_data = Instant::now();
        loop {
            if !self.pending.is_empty() {
                last_data = Instant::now();
                let received = std::mem::take(&mut self.pending);
                print!("{}", latin1(&received));
                let _ = io::stdout().flush();
                output.extend_from_slice(&received);
                if find_bytes(&output, b"boot>").is_some() {
                    break;
                }
            }
            if Instant::now() >= deadline
                || (!output.is_empty() && last_data.elapsed() >= Duration::from_millis(500))
            {
                break;
            }
            self.read_more(false)?;
        }
        if !output.is_empty() && !output.ends_with(b"\n") {
            println!();
        }
        Ok(output)
    }

    fn read_until(&mut self, marker: &[u8], timeout: Duration) -> AppResult<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(index) = find_bytes(&self.pending, marker) {
                let before = self.pending[..index].to_vec();
                self.pending.drain(..index + marker.len());
                return Ok(before);
            }
            if Instant::now() >= deadline {
                return Err(format!("timeout waiting for marker '{}'", latin1(marker)));
            }
            self.read_more(false)?;
        }
    }

    fn read_exact_pending(
        &mut self,
        size: usize,
        timeout: Duration,
        what: &str,
    ) -> AppResult<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        while self.pending.len() < size {
            if Instant::now() >= deadline {
                return Err(format!(
                    "timeout reading {what}: received {}/{} bytes",
                    self.pending.len(),
                    size
                ));
            }
            self.read_more(false)?;
        }
        Ok(self.pending.drain(..size).collect())
    }

    fn read_exact_with_progress(
        &mut self,
        size: usize,
        timeout: Duration,
        started: Instant,
    ) -> AppResult<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        print_progress(0, size, started, "Receive");
        while self.pending.len() < size {
            if Instant::now() >= deadline {
                return Err(format!(
                    "timeout reading binary payload: received {}/{} bytes",
                    self.pending.len(),
                    size
                ));
            }
            self.read_more(false)?;
            let received = self.pending.len().min(size);
            print_progress(received, size, started, "Receive");
        }
        Ok(self.pending.drain(..size).collect())
    }

    fn read_more(&mut self, trace_bytes: bool) -> AppResult<usize> {
        let mut buffer = [0u8; 4096];
        match self.io.read(&mut buffer) {
            Ok(count) => {
                if count > 0 {
                    if self.verbose && trace_bytes {
                        eprintln!("    Rx {} bytes: {}", count, hex(&buffer[..count.min(32)]));
                    }
                    self.pending.extend_from_slice(&buffer[..count]);
                } else {
                    thread::sleep(Duration::from_millis(1));
                }
                Ok(count)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(0)
            }
            Err(error) => Err(format!("serial read failed: {error}")),
        }
    }

    fn send_chunks(&mut self, data: &[u8], chunk_size: usize, label: &str) -> AppResult<()> {
        let started = Instant::now();
        print_progress(0, data.len(), started, label);
        for (index, chunk) in data.chunks(chunk_size).enumerate() {
            self.write(chunk)?;
            self.flush()?;
            let sent = ((index + 1) * chunk_size).min(data.len());
            print_progress(sent, data.len(), started, label);
        }
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> AppResult<()> {
        self.io
            .write_all(data)
            .map_err(|error| format!("serial write failed: {error}"))
    }

    fn flush(&mut self) -> AppResult<()> {
        self.io
            .flush()
            .map_err(|error| format!("serial drain failed: {error}"))
    }

    fn log(&self, message: &str) {
        if self.verbose {
            println!("[*] {message}");
        }
    }
}

pub fn parse_number(value: &str) -> AppResult<usize> {
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse::<u64>()
    }
    .map_err(|_| {
        format!("invalid number '{value}' (expected decimal or 0x-prefixed hexadecimal)")
    })?;
    usize::try_from(parsed).map_err(|_| format!("number '{value}' is too large for this platform"))
}

pub fn gx_checksum(data: &[u8]) -> u32 {
    data.iter().enumerate().fold(0u32, |sum, (index, byte)| {
        sum.wrapping_add(u32::from(GX_KEY[index % GX_KEY.len()] ^ byte))
    })
}

fn find_handshake(data: &[u8]) -> Option<(usize, usize)> {
    // IPL noise varies, so anchor on 0x58 and accept a valid prefix in the
    // preceding three bytes, matching the reference's tolerant detector.
    for (index, byte) in data.iter().enumerate() {
        if *byte != 0x58 || index < 2 {
            continue;
        }
        let start = index.saturating_sub(3);
        if matches!(data[start], 0x00 | 0xb0 | 0xb8) {
            return Some((start, index + 1));
        }
    }
    None
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn find_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

fn find_token(haystack: &[u8], token: &[u8]) -> Option<usize> {
    haystack
        .windows(token.len())
        .enumerate()
        .position(|(start, window)| {
            if !window.eq_ignore_ascii_case(token) {
                return false;
            }
            let before = start == 0 || !haystack[start - 1].is_ascii_alphanumeric();
            let end = start + token.len();
            let after = end == haystack.len() || !haystack[end].is_ascii_alphanumeric();
            before && after
        })
}

fn find_tolerant_runget(data: &[u8], max_gap: usize) -> Option<usize> {
    let pattern = b"RUNGET";
    let mut pattern_index = 0;
    let mut last = None;
    for (index, byte) in data.iter().enumerate() {
        if byte.to_ascii_uppercase() != pattern[pattern_index] {
            continue;
        }
        if let Some(previous) = last {
            let gap = &data[previous + 1..index];
            if gap.len() > max_gap || gap.iter().any(u8::is_ascii_alphanumeric) {
                pattern_index = 0;
                last = None;
                continue;
            }
        }
        last = Some(index);
        pattern_index += 1;
        if pattern_index == pattern.len() {
            return last;
        }
    }
    None
}

fn find_ordered_runget(data: &[u8], max_gap: usize) -> Option<usize> {
    let pattern = b"RUNGET";
    let mut pattern_index = 0;
    let mut first = None;
    let mut last = None;
    for (index, byte) in data.iter().enumerate() {
        if byte.to_ascii_uppercase() != pattern[pattern_index] {
            continue;
        }
        if let Some(previous) = last {
            if index - previous > max_gap {
                pattern_index = 0;
                first = None;
                last = None;
                continue;
            }
        } else {
            first = Some(index);
        }
        last = Some(index);
        pattern_index += 1;
        if pattern_index == pattern.len() {
            return first;
        }
    }
    None
}

fn clean_text_response(command: &str, response: &[u8]) -> String {
    let text = latin1(response);
    let mut lines = text.lines();
    let mut result = Vec::new();
    let mut found_echo = false;
    for line in lines.by_ref() {
        if !found_echo && line.contains(command) {
            found_echo = true;
            continue;
        }
        if found_echo {
            result.push(line.trim());
        }
    }
    if found_echo {
        result.join("\n").trim().to_owned()
    } else {
        text.trim().to_owned()
    }
}

fn latin1(data: &[u8]) -> String {
    data.iter().map(|byte| char::from(*byte)).collect()
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn print_progress(done: usize, total: usize, started: Instant, label: &str) {
    let percent = done.saturating_mul(100) / total.max(1);
    let bar_width = 28;
    let filled = done.saturating_mul(bar_width) / total.max(1);
    let bar = format!("{}{}", "=".repeat(filled), " ".repeat(bar_width - filled));
    let elapsed = started.elapsed().as_secs_f64();
    let speed = if elapsed > 0.0 {
        done as f64 / elapsed
    } else {
        0.0
    };
    print!(
        "\r  {label}: [{bar}] {percent:3}% ({:.1} KiB/s)",
        speed / 1024.0
    );
    if done == total {
        println!();
    } else {
        let _ = io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    #[derive(Default)]
    struct MockTransport {
        reads: VecDeque<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl MockTransport {
        fn with_reads(reads: &[&[u8]]) -> Self {
            Self {
                reads: reads.iter().map(|data| data.to_vec()).collect(),
                writes: Vec::new(),
            }
        }
    }

    fn boot_image() -> BootImage {
        let mut data = vec![0u8; 0x2020];
        data[..4].copy_from_slice(b"toob");
        data[4..6].copy_from_slice(&1u16.to_le_bytes());
        data[6..8].copy_from_slice(&0x6701u16.to_le_bytes());
        data[8..12].copy_from_slice(&115_200u32.to_le_bytes());
        for (index, byte) in data[0x20..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        BootImage::parse("mock.boot", data).unwrap()
    }

    impl Transport for MockTransport {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(mut data) = self.reads.pop_front() else {
                return Err(io::ErrorKind::TimedOut.into());
            };
            let count = data.len().min(buffer.len());
            buffer[..count].copy_from_slice(&data[..count]);
            if data.len() > count {
                data.drain(..count);
                self.reads.push_front(data);
            }
            Ok(count)
        }

        fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
            self.writes.extend_from_slice(data);
            Ok(())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn discard_buffers(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn set_dtr(&mut self, _state: bool) -> io::Result<()> {
            Ok(())
        }
        fn set_rts(&mut self, _state: bool) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn detects_fragmented_handshake() {
        let io = MockTransport::with_reads(&[&[0xaa, 0xb8], &[0xb0], &[0xff, 0x58]]);
        let mut session = Session::new(io, false);
        assert_eq!(
            session
                .wait_for_handshake(Duration::from_millis(50))
                .unwrap(),
            [0xb8, 0xb0, 0xff, 0x58]
        );
    }

    #[test]
    fn detects_handshake_after_ipl_noise() {
        let io = MockTransport::with_reads(&[&[0x19, 0x72, 0xb8], &[0xb0, 0xff], &[0x58]]);
        let mut session = Session::new(io, false);
        assert_eq!(
            session
                .wait_for_handshake(Duration::from_millis(50))
                .unwrap(),
            [0xb8, 0xb0, 0xff, 0x58]
        );
    }

    #[test]
    fn detects_runget_with_separators_and_ipl_noise() {
        let mut session = Session::new(MockTransport::with_reads(&[b"19RUkgd:3\r\nNGET"]), false);
        session.wait_for_runget(Duration::from_millis(50)).unwrap();
    }

    #[test]
    fn binary_read_preserves_payload_and_following_markers_in_one_read() {
        let io = MockTransport::with_reads(&[
            b"boot>",
            b"serialdump BOOT 4\r\n~st",
            b"a~\x01\x02\x03\x04~crc~\x78\x56\x34\x12~fin~",
        ]);
        let mut session = Session::new(io, false);
        let (data, crc) = session.binary_read("serialdump BOOT 4", 4).unwrap();
        assert_eq!(data, [1, 2, 3, 4]);
        assert_eq!(crc, Some(0x12345678));
    }

    #[test]
    fn complete_boot_writes_exact_stage_boundaries() {
        let io = MockTransport::with_reads(&[
            &[0xb0, 0xb0],
            &[0x58],
            b"RUN",
            b"GET",
            b"board output\r\nboot>",
        ]);
        let mut session = Session::new(io, false);
        let image = boot_image();
        let output = session.boot(&image, false, false).unwrap();
        assert!(find_bytes(&output, b"boot>").is_some());

        let writes = &session.io.writes;
        assert_eq!(&writes[..5], image.stage1_header());
        assert_eq!(
            &writes[5..5 + image.stage1_payload().len()],
            image.stage1_payload()
        );
        let stage1_end = 5 + image.stage1_payload().len();
        assert_eq!(&writes[stage1_end..stage1_end + 4], b"boot");

        let stage2_start = stage1_end + 4;
        let (metadata, content) = image.stage2();
        assert_eq!(&writes[stage2_start..stage2_start + 8], &metadata);
        assert_eq!(&writes[stage2_start + 8..], &content);
    }

    #[test]
    fn binary_write_sends_custom_checksum_after_device_request() {
        let io = MockTransport::with_reads(&[
            b"boot>",
            b"gx_otp write 0 5\r\n~sta~",
            b"~crc~",
            b"~fin~",
        ]);
        let mut session = Session::new(io, false);
        let data = [1, 2, 3, 4, 5];
        session
            .binary_write("gx_otp write 0 5", &data, Duration::from_secs(1), false)
            .unwrap();

        let writes = &session.io.writes;
        assert!(find_bytes(writes, b"gx_otp write 0 5\n").is_some());
        let data_at = find_bytes(writes, &data).unwrap();
        assert_eq!(
            &writes[data_at + data.len()..],
            &gx_checksum(&data).to_be_bytes()
        );
    }

    #[test]
    fn checksum_is_chunk_independent_and_big_endian_ready() {
        let data = [0x00, 0x11, 0x22, 0x33, 0xff];
        let expected: u32 =
            0x12u32 + (0x34u32 ^ 0x11) + (0x56u32 ^ 0x22) + (0x78u32 ^ 0x33) + (0x12u32 ^ 0xff);
        assert_eq!(gx_checksum(&data), expected);
        assert_eq!(gx_checksum(&data).to_be_bytes(), expected.to_be_bytes());
    }

    #[test]
    fn parses_decimal_and_hex_numbers() {
        assert_eq!(parse_number("4096").unwrap(), 4096);
        assert_eq!(parse_number("0x1000").unwrap(), 4096);
        assert!(parse_number("wat").is_err());
    }
}
