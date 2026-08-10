use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::{AppResult, commands::Command};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum TransferMode {
    #[default]
    S,
    Nns,
}

#[derive(Debug, Parser)]
#[command(
    name = "gxdl-rs",
    version,
    about = "Open-source NationalChip GX bootloader and flash utility",
    arg_required_else_help = true,
    after_help = "The target must normally be power-cycled after the tool starts.\n\
                  Embedded loaders are available only in builds made with --features embed-loaders."
)]
pub struct Cli {
    /// Custom .boot loader file. Takes precedence over --model.
    #[arg(short = 'b', long)]
    pub boot: Option<PathBuf>,

    /// Embedded loader filename or filename without the .boot suffix.
    #[arg(short = 'm', long)]
    pub model: Option<String>,

    /// Serial device, for example /dev/ttyUSB0 or COM3.
    #[arg(short = 'd', long)]
    pub device: Option<PathBuf>,

    /// Bootloader command and its arguments.
    #[arg(short = 'c', long)]
    pub command: Option<String>,

    /// s uploads a loader; nns uses a device already at the boot prompt.
    #[arg(short = 't', long, value_enum, default_value_t)]
    pub transfer_mode: TransferMode,

    /// Skip flash erase confirmation prompts.
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// UART baud rate.
    #[arg(long, default_value_t = 115_200)]
    pub baud: u32,

    /// Print protocol diagnostics.
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Pulse DTR before waiting for the BootROM handshake.
    #[arg(long)]
    pub reset_dtr: bool,

    /// Pulse RTS before waiting for the BootROM handshake.
    #[arg(long)]
    pub reset_rts: bool,

    /// Test a physical TX-to-RX loopback instead of contacting a GX device.
    #[arg(long)]
    pub loopback_test: bool,

    /// List loaders compiled into this executable.
    #[arg(long)]
    pub list_loaders: bool,
}

impl Cli {
    pub fn parsed_command(&self) -> AppResult<Option<Command>> {
        self.command.as_deref().map(Command::parse).transpose()
    }

    pub fn validate(&self, command: Option<&Command>) -> AppResult<()> {
        if self.list_loaders {
            return Ok(());
        }

        if self.loopback_test {
            if self.device.is_none() {
                return Err("--loopback-test requires -d/--device".into());
            }
            return Ok(());
        }

        if matches!(command, Some(Command::Compare { .. })) {
            return Ok(());
        }

        if self.device.is_none() {
            return Err("this operation requires -d/--device".into());
        }

        if self.transfer_mode == TransferMode::S && self.boot.is_none() && self.model.is_none() {
            return Err("transfer mode 's' requires -b/--boot or -m/--model".into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn custom_boot_takes_no_cli_conflict_with_model() {
        let cli =
            Cli::try_parse_from(["gxdl-rs", "-d", "port", "-b", "a.boot", "-m", "x"]).unwrap();
        cli.validate(None).unwrap();
        assert_eq!(cli.boot, Some(PathBuf::from("a.boot")));
    }

    #[test]
    fn nns_needs_device_but_not_loader() {
        let cli = Cli::try_parse_from(["gxdl-rs", "-d", "port", "-t", "nns"]).unwrap();
        cli.validate(None).unwrap();
    }

    #[test]
    fn compare_needs_neither_device_nor_loader() {
        let cli = Cli::try_parse_from(["gxdl-rs", "-c", "compare a b"]).unwrap();
        let command = cli.parsed_command().unwrap();
        cli.validate(command.as_ref()).unwrap();
    }

    #[test]
    fn bare_invocation_displays_help() {
        let error = Cli::try_parse_from(["gxdl-rs"]).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        assert!(error.to_string().contains("Usage: gxdl-rs [OPTIONS]"));
    }
}
