use std::{io, path::Path, thread, time::Duration};

use serial2::{CharSize, FlowControl, Parity, SerialPort, Settings, StopBits};

use crate::AppResult;

pub trait Transport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize>;
    fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
    fn discard_buffers(&mut self) -> io::Result<()>;
    fn set_dtr(&mut self, state: bool) -> io::Result<()>;
    fn set_rts(&mut self, state: bool) -> io::Result<()>;
}

pub struct SerialTransport {
    port: SerialPort,
}

impl SerialTransport {
    pub fn open(path: &Path, baud: u32) -> AppResult<Self> {
        let mut port = SerialPort::open(path, |mut settings: Settings| {
            settings.set_raw();
            settings.set_baud_rate(baud)?;
            settings.set_char_size(CharSize::Bits8);
            settings.set_stop_bits(StopBits::One);
            settings.set_parity(Parity::None);
            settings.set_flow_control(FlowControl::None);

            #[cfg(unix)]
            {
                let termios = settings.as_termios_mut();
                termios.c_iflag = libc::INPCK as _;
                termios.c_oflag = 0;
                termios.c_lflag = 0;
                termios.c_cflag |=
                    (libc::CREAD | libc::CLOCAL | libc::HUPCL | libc::CS8) as libc::tcflag_t;
            }
            Ok(settings)
        })
        .map_err(|error| format!("failed to open serial device '{}': {error}", path.display()))?;

        port.set_read_timeout(Duration::from_millis(100))
            .map_err(|error| format!("failed to set serial read timeout: {error}"))?;
        port.set_write_timeout(Duration::from_secs(5))
            .map_err(|error| format!("failed to set serial write timeout: {error}"))?;
        port.set_dtr(false)
            .map_err(|error| format!("failed to lower DTR: {error}"))?;
        port.set_rts(false)
            .map_err(|error| format!("failed to lower RTS: {error}"))?;
        port.discard_buffers()
            .map_err(|error| format!("failed to flush serial buffers: {error}"))?;

        Ok(Self { port })
    }
}

impl Transport for SerialTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.port.read(buffer)
    }

    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.port.write_all(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.port.flush()
    }

    fn discard_buffers(&mut self) -> io::Result<()> {
        self.port.discard_buffers()
    }

    fn set_dtr(&mut self, state: bool) -> io::Result<()> {
        self.port.set_dtr(state)
    }

    fn set_rts(&mut self, state: bool) -> io::Result<()> {
        self.port.set_rts(state)
    }
}

pub fn pulse_reset<T: Transport>(io: &mut T, dtr: bool, rts: bool) -> AppResult<()> {
    if dtr {
        io.set_dtr(true)
            .map_err(|error| format!("failed to assert DTR: {error}"))?;
        thread::sleep(Duration::from_millis(100));
        io.set_dtr(false)
            .map_err(|error| format!("failed to lower DTR: {error}"))?;
    }
    if rts {
        io.set_rts(true)
            .map_err(|error| format!("failed to assert RTS: {error}"))?;
        thread::sleep(Duration::from_millis(100));
        io.set_rts(false)
            .map_err(|error| format!("failed to lower RTS: {error}"))?;
    }
    if dtr || rts {
        thread::sleep(Duration::from_millis(200));
        io.discard_buffers()
            .map_err(|error| format!("failed to flush after reset: {error}"))?;
    }
    Ok(())
}

pub fn loopback(path: &Path, baud: u32) -> AppResult<()> {
    let mut transport = SerialTransport::open(path, baud)?;
    const PAYLOAD: &[u8] = b"LOOPBACK_TEST_12345";
    transport
        .write_all(PAYLOAD)
        .map_err(|error| format!("loopback write failed: {error}"))?;
    transport
        .flush()
        .map_err(|error| format!("loopback drain failed: {error}"))?;
    thread::sleep(Duration::from_millis(100));

    let mut received = Vec::with_capacity(PAYLOAD.len());
    let mut buffer = [0u8; 64];
    while received.len() < PAYLOAD.len() {
        match transport.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => received.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(error) => return Err(format!("loopback read failed: {error}")),
        }
    }
    if received == PAYLOAD {
        println!("[+] Loopback OK: sent and received {} bytes", PAYLOAD.len());
        Ok(())
    } else {
        Err(format!(
            "loopback failed: sent {}, received {}",
            hex(PAYLOAD),
            if received.is_empty() {
                "nothing".into()
            } else {
                hex(&received)
            }
        ))
    }
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|byte| format!("{byte:02x}")).collect()
}
