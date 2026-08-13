# gxdl-rs

`gxdl-rs` is an open-source NationalChip GX BootROM uploader and bootloader command client. It is a Rust implementation of the reverse-engineered protocol documented in [`reference/PROTOCOL.md`](reference/PROTOCOL.md) and supports the command surface of [`libre-gxdl`](https://github.com/matu6968/libre-gxdl).

The tool has been designed around GX set-top-box SoCs such as Gemini, Cygnus, Sirius, Taurus, Canopus, and Vega. Only limited hardware combinations have been independently tested, so keep a verified flash backup and a recovery method available before writing anything.

## Building

A normal build accepts a loader from the filesystem:

```console
cargo build --release
target/release/gxdl-rs -d /dev/ttyUSB0 -b loader.boot
```

Bundled NationalChip loader binaries are deliberately excluded from default builds. Enable the `embed-loaders` feature to compile them into the executable:

```console
cargo build --release --features embed-loaders
target/release/gxdl-rs --list-loaders
target/release/gxdl-rs -d /dev/ttyUSB0 -m gemini-6702H5-sflash-24M
```

`-m/--model` accepts either the listed filename or the same name without `.boot`. `-b/--boot` always selects a custom loader and takes precedence if both options are present.

The code uses exact raw termios settings, including `INPCK`, on Unix platforms. Windows uses the closest raw 8N1, no-flow-control configuration exposed by `serial2`; serial devices are normally named `COM3`, `COM4`, and so on.

## Library use

The package builds both the `gxdl-rs` executable and the `gxdl_rs` Rust library. `GxUploader` is the high-level equivalent of the Python reference's `GXUploader` class:

```rust
use gxdl_rs::{AppResult, GxUploader};

fn main() -> AppResult<()> {
    let uploader = GxUploader::new("/dev/ttyUSB0")
        .baudrate(115_200)
        .verbose(true)
        .reset_dtr(false)
        .skip_warnings(false);

    // The returned connection keeps the boot> session open.
    let mut device = uploader.upload_file("gemini.boot")?;
    device.execute_str("flash badinfo")?;
    Ok(())
}
```

Use `upload_model("gemini-6702H5-sflash-24M")` in a build with `embed-loaders`, or `attach()` to connect to a device already at `boot>`. `GxConnection::execute` accepts the typed `Command` enum, while `execute_str` accepts the same command strings as the standalone application.

The public `Session<T>` and `Transport` APIs support custom serial transports and lower-level protocol access:

```rust
let mut device = GxConnection::from_transport(my_transport, false);
device.session_mut().text_command("flash badinfo", timeout)?;
```

An uppercase `GXUploader` type alias is also exported for users migrating directly from the Python class name.

### Runnable library examples

Cargo examples under `examples/` compile as separate applications while using the public `gxdl_rs` library API.

Upload a custom loader file:

```console
cargo run --release --example boot_file -- \
  -d /dev/ttyUSB0 -b src/loaders/gemini-6702H5-sflash-24M.boot -v
```

Upload an embedded loader and execute a command:

```console
cargo run --release --features embed-loaders --example embedded_loader -- \
  -d /dev/ttyUSB0 -m gemini-6702H5-sflash-24M \
  -c "flash badinfo" -v
```

Compile examples without running them:

```console
cargo build --examples
cargo build --examples --features embed-loaders
```

## Basic use

Start the tool before power-cycling the target, because the BootROM handshake window is short:

```console
# Boot using a custom loader
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -v

# Pulse a control line to reset boards wired for it
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot --reset-dtr

# Reuse a loader already sitting at its boot> prompt
gxdl-rs -d /dev/ttyUSB0 -t nns -c "flash badinfo"

# Check the host serial adapter with TX physically connected to RX
gxdl-rs -d /dev/ttyUSB0 --loopback-test
```

Transfer mode `s`, the default, sends a selected loader. Transfer mode `nns` skips BootROM upload and therefore needs only a serial device.

The uploader chooses the Stage 1 packet layout from the loader header's chip ID: `0x6612` transfers the 0x4000-byte layout, `0x6616`, `0x3211`, `0x6701`, and `0x6705` use the 0x2000-byte layout, and other IDs use the 0x1000-byte layout. Stage 2 sends the `boot` continuation marker from Stage 1, then a little-endian 32-bit additive checksum, the original file size, and transformed content after `RUNGET`.

## Commands

Bootloader commands are passed as one quoted value through `-c/--command`. Arguments are separated on whitespace, matching the reference utility, so filenames containing whitespace are not supported.

### Serial and USB transfer

```console
# Read BOOT to a host file
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot \
  -c "serialdump BOOT 65536 boot.bin"

# Read an entire 4 MiB flash
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot \
  -c "serialdump 0x0 0x400000 full-flash.bin"

# Write a host file over UART
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot \
  -c "serialdown LOGO logo.bin"

# Have the target write/read a file on attached USB storage
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot \
  -c "usbdump KERNEL 2752512 kernel.bin"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot \
  -c "usbdown LOGO logo.bin"
```

`serialdump` records the four-byte CRC reported by the device in verbose output, but does not verify it because its algorithm is not documented. Serial writes use the GX repeating-key XOR-sum required by the loader, not CRC32.

### OTP operations

```console
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "gx_otp tread 0 32"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "gx_otp read 0 64 gx-otp.bin"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "gx_otp write 0 gx-otp.bin"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "gx_otp twrite 0 01234567"

gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "sflash_otp status"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "sflash_otp getregion"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "sflash_otp read 0 64 spi-otp.bin"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "sflash_otp write 0 spi-otp.bin"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "sflash_otp erase"
```

OTP writes can be permanent. SPI OTP transfer behavior is based on the reference utility's partially reverse-engineered implementation and can vary by flash model.

### Flash management and host utilities

```console
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "flash badinfo"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "flash erase LOGO"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "flash erase nospread 0x10000 65536"
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot -c "flash eraseall"

# Host-only; no device or loader is required
gxdl-rs -c "compare first.bin second.bin"
```

Like the Python reference, `flash erase` and `flash eraseall` ask for confirmation. Pass `-y/--yes` for unattended use. Other write commands do not prompt, so merely invoking them authorizes the write.

## Configuration files

`load_conf_down` reads one command per line. Empty lines and lines beginning with `#` are ignored. Supported entries are `serialdump`, `serialdown`, `usbdump`, `usbdown`, and `flash` commands.

```text
# factory-download.conf
serialdown BOOT boot.bin
usbdown KERNEL kernel.bin
flash erase DATA
```

```console
gxdl-rs -d /dev/ttyUSB0 -b gemini.boot \
  -c "load_conf_down factory-download.conf serial"
```

The transport and optional transport-path arguments are accepted for vendor command compatibility. Individual config lines determine the actual operation.

## Troubleshooting

- A handshake timeout usually means the target was not reset during the short BootROM window, TX/RX are swapped, ground is missing, or the selected port is wrong.
- A `RUNGET` timeout indicates a damaged or incompatible loader, serial corruption, or an incorrect board model. Confirm the loader has `toob` magic and try a clean power cycle.
- If `nns` fails, confirm the target is already displaying `boot>` and that no other process owns the serial port.
- DTR and RTS wiring differs between adapters. Use reset flags only when the board is known to connect those signals appropriately.
- USB commands operate on storage attached to the target, not the host computer, and commonly require FAT32.

## License

Licensed under either of:

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
   https://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   https://opensource.org/license/mit)

at your option excluding original loaders present in `src/loaders` which are (c) NationalChip.
