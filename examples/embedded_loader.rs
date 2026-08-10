use std::path::PathBuf;

use clap::Parser;
use gxdl_rs::{AppResult, GxUploader};

/// Upload a GX loader embedded through the embed-loaders feature.
#[derive(Debug, Parser)]
struct Args {
    /// Serial device, for example /dev/ttyUSB0 or COM3.
    #[arg(short = 'd', long)]
    device: PathBuf,

    /// Embedded loader filename or extensionless stem.
    #[arg(short = 'm', long)]
    model: String,

    /// Optional command to execute after reaching boot>.
    #[arg(short = 'c', long)]
    command: Option<String>,

    /// UART baud rate.
    #[arg(long, default_value_t = 115_200)]
    baud: u32,

    /// Print protocol diagnostics.
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Pulse DTR before waiting for the BootROM handshake.
    #[arg(long)]
    reset_dtr: bool,

    /// Pulse RTS before waiting for the BootROM handshake.
    #[arg(long)]
    reset_rts: bool,
}

fn main() -> AppResult<()> {
    let args = Args::parse();
    let uploader = GxUploader::new(args.device)
        .baudrate(args.baud)
        .verbose(args.verbose)
        .reset_dtr(args.reset_dtr)
        .reset_rts(args.reset_rts);

    let mut device = uploader.upload_model(&args.model)?;
    if let Some(command) = args.command {
        device.execute_str(&command)?;
    }
    Ok(())
}
