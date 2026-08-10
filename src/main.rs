use std::process::ExitCode;

use clap::Parser;
use gxdl_rs::{
    AppResult, GxUploader,
    cli::{Cli, TransferMode},
    commands, loader,
    serial::loopback,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[!] {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> AppResult<()> {
    let cli = Cli::parse();
    let command = cli.parsed_command()?;
    cli.validate(command.as_ref())?;

    if cli.list_loaders {
        for name in loader::embedded_names()? {
            println!("{name}");
        }
        return Ok(());
    }

    if cli.loopback_test {
        println!("[*] Serial loopback test: connect TX directly to RX first");
        return loopback(cli.device.as_deref().expect("validated device"), cli.baud);
    }

    if let Some(command) = command.as_ref()
        && commands::execute_host(command)?
    {
        return Ok(());
    }

    let device = cli.device.as_deref().expect("validated device");
    let uploader = GxUploader::new(device)
        .baudrate(cli.baud)
        .verbose(cli.verbose)
        .reset_dtr(cli.reset_dtr)
        .reset_rts(cli.reset_rts)
        .skip_warnings(cli.yes);
    if cli.verbose {
        println!("[*] Opening {} at {} baud", device.display(), cli.baud);
    }

    let mut connection = match cli.transfer_mode {
        TransferMode::S => {
            let image = loader::resolve(cli.boot.as_deref(), cli.model.as_deref())?;
            uploader.upload(&image)?
        }
        TransferMode::Nns => {
            let connection = uploader.attach().map_err(|_| {
                "device is not at the boot> prompt; cannot use transfer mode nns".to_string()
            })?;
            println!("[+] Connected to existing bootloader session");
            connection
        }
    };

    if let Some(command) = command.as_ref() {
        connection.execute(command)?;
    }
    Ok(())
}
