use std::{path::Path, time::Duration};

use crate::{
    AppResult,
    commands::{self, Command},
    loader::{self, BootImage},
    protocol::Session,
    serial::{SerialTransport, Transport},
};

/// High-level configuration and connection factory for GX devices.
///
/// This is the Rust equivalent of the reference Python `GXUploader` class.
/// Configuration methods use the builder pattern, and uploading returns a
/// [`GxConnection`] that retains the live `boot>` session for further commands.
#[derive(Clone, Debug)]
pub struct GxUploader {
    device: std::path::PathBuf,
    baudrate: u32,
    verbose: bool,
    reset_dtr: bool,
    reset_rts: bool,
    skip_warnings: bool,
}

impl GxUploader {
    /// Create an uploader for a serial device using 115200 baud.
    pub fn new(device: impl Into<std::path::PathBuf>) -> Self {
        Self {
            device: device.into(),
            baudrate: 115_200,
            verbose: false,
            reset_dtr: false,
            reset_rts: false,
            skip_warnings: false,
        }
    }

    /// Override the UART baud rate.
    pub fn baudrate(mut self, baudrate: u32) -> Self {
        self.baudrate = baudrate;
        self
    }

    /// Enable or disable protocol diagnostics.
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Configure whether DTR is pulsed before BootROM synchronization.
    pub fn reset_dtr(mut self, enabled: bool) -> Self {
        self.reset_dtr = enabled;
        self
    }

    /// Configure whether RTS is pulsed before BootROM synchronization.
    pub fn reset_rts(mut self, enabled: bool) -> Self {
        self.reset_rts = enabled;
        self
    }

    /// Skip the confirmation prompts used by flash erase commands.
    pub fn skip_warnings(mut self, enabled: bool) -> Self {
        self.skip_warnings = enabled;
        self
    }

    pub fn device(&self) -> &Path {
        &self.device
    }

    pub fn configured_baudrate(&self) -> u32 {
        self.baudrate
    }

    /// Open the serial device without booting or checking its current state.
    pub fn open(&self) -> AppResult<GxConnection<SerialTransport>> {
        let transport = SerialTransport::open(&self.device, self.baudrate)?;
        Ok(GxConnection {
            session: Session::new(transport, self.verbose),
            skip_warnings: self.skip_warnings,
        })
    }

    /// Open a device that is already running a loader and verify `boot>`.
    pub fn attach(&self) -> AppResult<GxConnection<SerialTransport>> {
        let mut connection = self.open()?;
        connection
            .session
            .wait_for_prompt(Duration::from_secs(2))
            .map_err(|_| "device is not at the boot> prompt".to_string())?;
        Ok(connection)
    }

    /// Upload a parsed boot image and return its live command session.
    pub fn upload(&self, image: &BootImage) -> AppResult<GxConnection<SerialTransport>> {
        let mut connection = self.open()?;
        connection
            .session
            .boot(image, self.reset_dtr, self.reset_rts)?;
        Ok(connection)
    }

    /// Load a custom `.boot` file, upload it, and return its command session.
    pub fn upload_file(&self, path: impl AsRef<Path>) -> AppResult<GxConnection<SerialTransport>> {
        let image = BootImage::from_file(path.as_ref())?;
        self.upload(&image)
    }

    /// Upload a loader compiled in through the `embed-loaders` feature.
    ///
    /// Both its canonical `.boot` filename and its extensionless stem are
    /// accepted. Builds without the feature return a descriptive error.
    pub fn upload_model(&self, model: &str) -> AppResult<GxConnection<SerialTransport>> {
        let image = loader::resolve(None, Some(model))?;
        self.upload(&image)
    }
}

/// A live serial connection to a GX loader.
///
/// Use [`execute`](Self::execute) for typed commands, [`execute_str`](Self::execute_str)
/// for reference-compatible command strings, or [`session_mut`](Self::session_mut)
/// for direct access to the lower-level protocol API.
pub struct GxConnection<T: Transport = SerialTransport> {
    session: Session<T>,
    skip_warnings: bool,
}

impl<T: Transport> GxConnection<T> {
    /// Wrap a custom transport in a high-level connection.
    ///
    /// This is useful for applications with their own serial abstraction and
    /// for deterministic protocol tests.
    pub fn from_transport(transport: T, verbose: bool) -> Self {
        Self {
            session: Session::new(transport, verbose),
            skip_warnings: false,
        }
    }

    pub fn skip_warnings(mut self, enabled: bool) -> Self {
        self.skip_warnings = enabled;
        self
    }

    /// Execute a parsed bootloader command on this connection.
    pub fn execute(&mut self, command: &Command) -> AppResult<()> {
        commands::execute(&mut self.session, command, self.skip_warnings)
    }

    /// Parse and execute a reference-compatible command string.
    pub fn execute_str(&mut self, command: &str) -> AppResult<()> {
        let command = Command::parse(command)?;
        self.execute(&command)
    }

    pub fn session(&self) -> &Session<T> {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session<T> {
        &mut self.session
    }

    pub fn into_session(self) -> Session<T> {
        self.session
    }
}

/// Compatibility spelling for users coming from the Python `GXUploader` API.
#[allow(clippy::upper_case_acronyms)]
pub type GXUploader = GxUploader;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[derive(Default)]
    struct NoopTransport;

    impl Transport for NoopTransport {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::TimedOut.into())
        }
        fn write_all(&mut self, _data: &[u8]) -> io::Result<()> {
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
    fn uploader_builder_exposes_core_configuration() {
        let uploader = GxUploader::new("test-port").baudrate(230_400).verbose(true);
        assert_eq!(uploader.device(), Path::new("test-port"));
        assert_eq!(uploader.configured_baudrate(), 230_400);
    }

    #[test]
    fn custom_transport_connection_exposes_protocol_session() {
        let mut connection = GxConnection::from_transport(NoopTransport, false);
        let _session: &mut Session<NoopTransport> = connection.session_mut();
    }
}
