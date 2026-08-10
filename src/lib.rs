//! NationalChip GX BootROM uploader and bootloader command client.
//!
//! [`GxUploader`] is the high-level entry point for applications. The
//! [`protocol`], [`serial`], [`loader`], and [`commands`] modules remain public
//! for callers that need custom transports or lower-level protocol control.
//!
//! ```no_run
//! use gxdl_rs::{GxUploader, AppResult};
//!
//! fn main() -> AppResult<()> {
//!     let uploader = GxUploader::new("/dev/ttyUSB0").verbose(true);
//!     let mut device = uploader.upload_file("loader.boot")?;
//!     device.execute_str("flash badinfo")?;
//!     Ok(())
//! }
//! ```

pub mod cli;
pub mod commands;
pub mod loader;
pub mod protocol;
pub mod serial;
pub mod uploader;

pub use commands::Command;
pub use loader::BootImage;
pub use uploader::{GXUploader, GxConnection, GxUploader};

pub type AppResult<T> = Result<T, String>;
