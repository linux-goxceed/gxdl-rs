use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    AppResult,
    protocol::{Session, parse_number},
    serial::Transport,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    SerialDump {
        target: String,
        size: usize,
        output: PathBuf,
    },
    SerialDown {
        target: String,
        input: PathBuf,
    },
    UsbDump {
        target: String,
        size: usize,
        filename: String,
    },
    UsbDown {
        target: String,
        filename: String,
    },
    GxOtp(GxOtpCommand),
    SflashOtp(SflashOtpCommand),
    Flash(FlashCommand),
    Compare {
        source: PathBuf,
        destination: PathBuf,
    },
    LoadConfig {
        config: PathBuf,
        transport: String,
        transport_path: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GxOtpCommand {
    Read {
        address: usize,
        length: usize,
        output: PathBuf,
    },
    Tread {
        address: usize,
        length: usize,
    },
    Write {
        address: usize,
        input: PathBuf,
    },
    Twrite {
        address: usize,
        hex: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SflashOtpCommand {
    Status,
    GetRegion,
    Read {
        address: usize,
        length: usize,
        output: PathBuf,
    },
    Write {
        address: usize,
        input: PathBuf,
    },
    Erase,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlashCommand {
    Erase {
        target: String,
        length: Option<usize>,
        nospread: bool,
    },
    BadInfo,
    EraseAll,
}

impl Command {
    pub fn parse(input: &str) -> AppResult<Self> {
        let parts: Vec<_> = input.split_whitespace().collect();
        let (name, args) = parts
            .split_first()
            .ok_or_else(|| "empty command".to_string())?;
        match *name {
            "serialdump" => {
                require_count(name, args, 3)?;
                Ok(Self::SerialDump {
                    target: args[0].into(),
                    size: parse_number(args[1])?,
                    output: args[2].into(),
                })
            }
            "serialdown" => {
                require_count(name, args, 2)?;
                Ok(Self::SerialDown {
                    target: args[0].into(),
                    input: args[1].into(),
                })
            }
            "usbdump" => {
                require_count(name, args, 3)?;
                Ok(Self::UsbDump {
                    target: args[0].into(),
                    size: parse_number(args[1])?,
                    filename: args[2].into(),
                })
            }
            "usbdown" => {
                require_count(name, args, 2)?;
                Ok(Self::UsbDown {
                    target: args[0].into(),
                    filename: args[1].into(),
                })
            }
            "gx_otp" => Ok(Self::GxOtp(GxOtpCommand::parse(args)?)),
            "sflash_otp" => Ok(Self::SflashOtp(SflashOtpCommand::parse(args)?)),
            "flash" => Ok(Self::Flash(FlashCommand::parse(args)?)),
            "compare" => {
                require_count(name, args, 2)?;
                Ok(Self::Compare {
                    source: args[0].into(),
                    destination: args[1].into(),
                })
            }
            "load_conf_down" => {
                if !(2..=3).contains(&args.len()) {
                    return Err(
                        "usage: load_conf_down <config_file> <transport> [transport_path]".into(),
                    );
                }
                Ok(Self::LoadConfig {
                    config: args[0].into(),
                    transport: args[1].into(),
                    transport_path: args.get(2).map(|value| (*value).into()),
                })
            }
            _ => Err(format!(
                "unknown command '{name}'; available: serialdump, serialdown, usbdump, usbdown, gx_otp, sflash_otp, flash, compare, load_conf_down"
            )),
        }
    }

    fn allowed_in_config(&self) -> bool {
        matches!(
            self,
            Self::SerialDump { .. }
                | Self::SerialDown { .. }
                | Self::UsbDump { .. }
                | Self::UsbDown { .. }
                | Self::Flash(_)
        )
    }
}

impl GxOtpCommand {
    fn parse(args: &[&str]) -> AppResult<Self> {
        let (name, args) = args
            .split_first()
            .ok_or_else(|| "usage: gx_otp <read|tread|write|twrite> ...".to_string())?;
        match *name {
            "read" => {
                require_count("gx_otp read", args, 3)?;
                Ok(Self::Read {
                    address: parse_number(args[0])?,
                    length: parse_number(args[1])?,
                    output: args[2].into(),
                })
            }
            "tread" => {
                require_count("gx_otp tread", args, 2)?;
                Ok(Self::Tread {
                    address: parse_number(args[0])?,
                    length: parse_number(args[1])?,
                })
            }
            "write" => {
                require_count("gx_otp write", args, 2)?;
                Ok(Self::Write {
                    address: parse_number(args[0])?,
                    input: args[1].into(),
                })
            }
            "twrite" => {
                require_count("gx_otp twrite", args, 2)?;
                Ok(Self::Twrite {
                    address: parse_number(args[0])?,
                    hex: args[1].into(),
                })
            }
            _ => Err(format!("unknown gx_otp subcommand '{name}'")),
        }
    }
}

impl SflashOtpCommand {
    fn parse(args: &[&str]) -> AppResult<Self> {
        let (name, args) = args.split_first().ok_or_else(|| {
            "usage: sflash_otp <status|getregion|read|write|erase> ...".to_string()
        })?;
        match *name {
            "status" => {
                require_count("sflash_otp status", args, 0)?;
                Ok(Self::Status)
            }
            "getregion" => {
                require_count("sflash_otp getregion", args, 0)?;
                Ok(Self::GetRegion)
            }
            "read" => {
                require_count("sflash_otp read", args, 3)?;
                Ok(Self::Read {
                    address: parse_number(args[0])?,
                    length: parse_number(args[1])?,
                    output: args[2].into(),
                })
            }
            "write" => {
                require_count("sflash_otp write", args, 2)?;
                Ok(Self::Write {
                    address: parse_number(args[0])?,
                    input: args[1].into(),
                })
            }
            "erase" => {
                require_count("sflash_otp erase", args, 0)?;
                Ok(Self::Erase)
            }
            _ => Err(format!("unknown sflash_otp subcommand '{name}'")),
        }
    }
}

impl FlashCommand {
    fn parse(args: &[&str]) -> AppResult<Self> {
        let (name, args) = args
            .split_first()
            .ok_or_else(|| "usage: flash <erase|badinfo|eraseall> ...".to_string())?;
        match *name {
            "erase" => {
                let nospread = args.first() == Some(&"nospread");
                let args = if nospread { &args[1..] } else { args };
                if !(1..=2).contains(&args.len()) {
                    return Err("usage: flash erase [nospread] <partition|address> [length]".into());
                }
                Ok(Self::Erase {
                    target: args[0].into(),
                    length: args.get(1).map(|value| parse_number(value)).transpose()?,
                    nospread,
                })
            }
            "badinfo" => {
                require_count("flash badinfo", args, 0)?;
                Ok(Self::BadInfo)
            }
            "eraseall" => {
                require_count("flash eraseall", args, 0)?;
                Ok(Self::EraseAll)
            }
            _ => Err(format!("unknown flash subcommand '{name}'")),
        }
    }
}

pub fn execute_host(command: &Command) -> AppResult<bool> {
    if let Command::Compare {
        source,
        destination,
    } = command
    {
        compare_files(source, destination)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn execute<T: Transport>(
    session: &mut Session<T>,
    command: &Command,
    yes: bool,
) -> AppResult<()> {
    match command {
        Command::SerialDump {
            target,
            size,
            output,
        } => {
            let wire = format!("serialdump {target} {size}");
            let (data, _) = session.binary_read(&wire, *size)?;
            write_output(output, &data)?;
        }
        Command::SerialDown { target, input } => {
            let data = read_input(input)?;
            println!("[*] Downloading {} bytes to {target}", data.len());
            let wire = format!("serialdown {target} {}", data.len());
            let response = session.binary_write(&wire, &data, Duration::from_secs(120), true)?;
            print_response("Serial download", &response);
        }
        Command::UsbDump {
            target,
            size,
            filename,
        } => {
            let wire = format!("usbdump {target} {size} {filename}");
            println!("[*] USB dump: {target} ({size} bytes) -> {filename}");
            let response = session.text_command(&wire, Duration::from_secs(120))?;
            print_response("USB dump", &response);
        }
        Command::UsbDown { target, filename } => {
            let wire = format!("usbdown {target} {filename}");
            println!("[*] USB download: {filename} -> {target}");
            println!("[!] WARNING: This will erase and write flash");
            let response = session.text_command(&wire, Duration::from_secs(300))?;
            print_response("USB download", &response);
        }
        Command::GxOtp(command) => execute_gx_otp(session, command)?,
        Command::SflashOtp(command) => execute_sflash_otp(session, command)?,
        Command::Flash(command) => execute_flash(session, command, yes)?,
        Command::Compare { .. } => {
            execute_host(command)?;
        }
        Command::LoadConfig {
            config,
            transport,
            transport_path,
        } => {
            println!(
                "[*] Loading config via {transport}: {}{}",
                config.display(),
                transport_path
                    .as_deref()
                    .map(|path| format!(" ({path})"))
                    .unwrap_or_default()
            );
            let commands = parse_config(config)?;
            for (line, command) in commands {
                if !command.allowed_in_config() {
                    return Err(format!(
                        "unsupported command on config line {line}: {command:?}"
                    ));
                }
                execute(session, &command, yes)?;
            }
            println!("[+] Config completed successfully");
        }
    }
    Ok(())
}

fn execute_gx_otp<T: Transport>(session: &mut Session<T>, command: &GxOtpCommand) -> AppResult<()> {
    match command {
        GxOtpCommand::Read {
            address,
            length,
            output,
        } => {
            let wire = format!("gx_otp read {address} {length}");
            let (data, _) = session.binary_read(&wire, *length)?;
            write_output(output, &data)?;
        }
        GxOtpCommand::Tread { address, length } => {
            let response = session.text_command(
                &format!("gx_otp tread {address} {length}"),
                Duration::from_secs(5),
            )?;
            print_response("GX OTP data", &response);
        }
        GxOtpCommand::Write { address, input } => {
            let data = read_input(input)?;
            println!(
                "[*] Writing {} bytes to GX OTP at 0x{address:X}",
                data.len()
            );
            let wire = format!("gx_otp write {address} {}", data.len());
            let response = session.binary_write(&wire, &data, Duration::from_secs(60), false)?;
            print_response("GX OTP write", &response);
        }
        GxOtpCommand::Twrite { address, hex } => {
            let response = session.text_command(
                &format!("gx_otp twrite {address} {hex}"),
                Duration::from_secs(30),
            )?;
            print_response("GX OTP twrite", &response);
        }
    }
    Ok(())
}

fn execute_sflash_otp<T: Transport>(
    session: &mut Session<T>,
    command: &SflashOtpCommand,
) -> AppResult<()> {
    match command {
        SflashOtpCommand::Status => {
            let response = session.text_command("sflash_otp status", Duration::from_secs(5))?;
            print_response("SPI flash OTP", &response);
        }
        SflashOtpCommand::GetRegion => {
            let response = session.text_command("sflash_otp getregion", Duration::from_secs(5))?;
            print_response("SPI flash OTP", &response);
        }
        SflashOtpCommand::Read {
            address,
            length,
            output,
        } => {
            let wire = format!("sflash_otp read {address} {length}");
            let (data, _) = session.binary_read(&wire, *length)?;
            write_output(output, &data)?;
        }
        SflashOtpCommand::Write { address, input } => {
            let data = read_input(input)?;
            println!(
                "[*] Writing {} bytes to SPI flash OTP at 0x{address:X}",
                data.len()
            );
            let wire = format!("sflash_otp write {address} {}", data.len());
            let response = session.binary_write(&wire, &data, Duration::from_secs(60), false)?;
            print_response("SPI flash OTP write", &response);
        }
        SflashOtpCommand::Erase => {
            println!("[*] Erasing SPI flash OTP region (dangerous)");
            let response = session.text_command("sflash_otp erase", Duration::from_secs(30))?;
            print_response("SPI flash OTP erase", &response);
        }
    }
    Ok(())
}

fn execute_flash<T: Transport>(
    session: &mut Session<T>,
    command: &FlashCommand,
    yes: bool,
) -> AppResult<()> {
    match command {
        FlashCommand::Erase {
            target,
            length,
            nospread,
        } => {
            if !yes
                && !confirm(&format!(
                    "This will erase {target}. Proceed with flash erase?"
                ))?
            {
                return Err("aborted".into());
            }
            let mut wire = String::from("flash erase");
            if *nospread {
                wire.push_str(" nospread");
            }
            wire.push(' ');
            wire.push_str(target);
            if let Some(length) = length {
                wire.push(' ');
                wire.push_str(&length.to_string());
            }
            let response = session.text_command(&wire, Duration::from_secs(120))?;
            print_response("Flash erase", &response);
        }
        FlashCommand::BadInfo => {
            let response = session.text_command("flash badinfo", Duration::from_secs(10))?;
            print_response("Flash bad block info", &response);
        }
        FlashCommand::EraseAll => {
            println!(
                "[!] WARNING: flash eraseall will erase all flash data and can brick the device"
            );
            if !yes && !confirm("Proceed with flash eraseall?")? {
                return Err("aborted".into());
            }
            let response = session.text_command("flash eraseall", Duration::from_secs(300))?;
            print_response("Flash eraseall", &response);
        }
    }
    Ok(())
}

fn parse_config(path: &Path) -> AppResult<Vec<(usize, Command)>> {
    let file = File::open(path)
        .map_err(|error| format!("failed to open config '{}': {error}", path.display()))?;
    let mut commands = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line =
            line.map_err(|error| format!("failed to read config line {}: {error}", index + 1))?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let command = Command::parse(trimmed)
            .map_err(|error| format!("config line {}: {error}", index + 1))?;
        commands.push((index + 1, command));
    }
    if commands.is_empty() {
        return Err(format!("config '{}' contains no commands", path.display()));
    }
    Ok(commands)
}

fn compare_files(source: &Path, destination: &Path) -> AppResult<()> {
    let mut first = File::open(source)
        .map_err(|error| format!("failed to open '{}': {error}", source.display()))?;
    let mut second = File::open(destination)
        .map_err(|error| format!("failed to open '{}': {error}", destination.display()))?;
    let first_len = first
        .metadata()
        .map_err(|error| format!("failed to inspect '{}': {error}", source.display()))?
        .len();
    let second_len = second
        .metadata()
        .map_err(|error| format!("failed to inspect '{}': {error}", destination.display()))?
        .len();
    if first_len != second_len {
        return Err(format!(
            "files differ in size: '{}' is {first_len} bytes, '{}' is {second_len} bytes",
            source.display(),
            destination.display()
        ));
    }

    let mut left = vec![0u8; 4 * 1024 * 1024];
    let mut right = vec![0u8; left.len()];
    let mut offset = 0u64;
    loop {
        let count = first
            .read(&mut left)
            .map_err(|error| format!("failed reading '{}': {error}", source.display()))?;
        second
            .read_exact(&mut right[..count])
            .map_err(|error| format!("failed reading '{}': {error}", destination.display()))?;
        if count == 0 {
            break;
        }
        if left[..count] != right[..count] {
            let index = left[..count]
                .iter()
                .zip(&right[..count])
                .position(|(left, right)| left != right)
                .expect("different chunks contain a different byte");
            return Err(format!(
                "files differ at offset 0x{:X}: '{}' has 0x{:02X}, '{}' has 0x{:02X}",
                offset + index as u64,
                source.display(),
                left[index],
                destination.display(),
                right[index]
            ));
        }
        offset += count as u64;
    }
    println!("[+] Files are identical ({first_len} bytes)");
    Ok(())
}

fn read_input(path: &Path) -> AppResult<Vec<u8>> {
    fs::read(path).map_err(|error| format!("failed to read '{}': {error}", path.display()))
}

fn write_output(path: &Path, data: &[u8]) -> AppResult<()> {
    fs::write(path, data)
        .map_err(|error| format!("failed to write '{}': {error}", path.display()))?;
    println!("[+] Wrote {} bytes to {}", data.len(), path.display());
    Ok(())
}

fn confirm(prompt: &str) -> AppResult<bool> {
    println!("[!] {prompt} (y/N)");
    std::io::stdout()
        .flush()
        .map_err(|error| format!("failed to print prompt: {error}"))?;
    let mut response = String::new();
    std::io::stdin()
        .read_line(&mut response)
        .map_err(|error| format!("failed to read confirmation: {error}"))?;
    Ok(matches!(
        response.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn print_response(label: &str, response: &str) {
    if response.trim().is_empty() {
        println!("[+] {label} command completed");
    } else {
        println!("[+] {label}:\n{}", response.trim());
    }
}

fn require_count(command: &str, args: &[&str], count: usize) -> AppResult<()> {
    if args.len() == count {
        Ok(())
    } else {
        Err(format!(
            "'{command}' expects {count} argument(s), got {}",
            args.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_major_command_families() {
        assert!(matches!(
            Command::parse("serialdump BOOT 0x100 dump.bin").unwrap(),
            Command::SerialDump { size: 256, .. }
        ));
        assert!(matches!(
            Command::parse("gx_otp tread 0x10 32").unwrap(),
            Command::GxOtp(GxOtpCommand::Tread {
                address: 16,
                length: 32
            })
        ));
        assert!(matches!(
            Command::parse("flash erase nospread 0x0 4096").unwrap(),
            Command::Flash(FlashCommand::Erase {
                nospread: true,
                length: Some(4096),
                ..
            })
        ));
        assert!(Command::parse("sflash_otp wat").is_err());
    }

    #[test]
    fn compare_reports_first_difference() {
        let root = std::env::temp_dir().join(format!("gxdl-rs-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first.bin");
        let second = root.join("second.bin");
        fs::write(&first, [1, 2, 3]).unwrap();
        fs::write(&second, [1, 9, 3]).unwrap();
        let error = compare_files(&first, &second).unwrap_err();
        assert!(error.contains("offset 0x1"));
        let _ = fs::remove_file(first);
        let _ = fs::remove_file(second);
        let _ = fs::remove_dir(root);
    }

    #[test]
    fn config_parser_includes_documented_usb_dump() {
        let path = std::env::temp_dir().join(format!(
            "gxdl-rs-config-test-{}-{}.conf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(
            &path,
            "# comment\n\nusbdump BOOT 256 boot.bin\nflash badinfo\n",
        )
        .unwrap();
        let commands = parse_config(&path).unwrap();
        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[0].1, Command::UsbDump { size: 256, .. }));
        assert!(matches!(
            commands[1].1,
            Command::Flash(FlashCommand::BadInfo)
        ));
        let _ = fs::remove_file(path);
    }
}
