use std::{fs, path::Path};

use crate::AppResult;

const MIN_BOOT_SIZE: usize = 0x2020;
const STAGE1_END: usize = 0x201c;

#[derive(Clone, Debug)]
pub struct BootImage {
    pub name: String,
    data: Vec<u8>,
    pub version: u16,
    pub chip: u16,
    pub baud: u32,
}

impl BootImage {
    pub fn parse(name: impl Into<String>, data: Vec<u8>) -> AppResult<Self> {
        let name = name.into();
        if data.len() < MIN_BOOT_SIZE {
            return Err(format!(
                "boot loader '{name}' is too small: {} bytes (need at least {MIN_BOOT_SIZE})",
                data.len()
            ));
        }
        if data.len() > u32::MAX as usize {
            return Err(format!(
                "boot loader '{name}' exceeds the protocol's 32-bit size field"
            ));
        }
        if data.get(..4) != Some(b"toob") {
            let magic = data.get(..4).unwrap_or(&data).to_vec();
            return Err(format!("invalid boot loader magic: {:02x?}", magic));
        }

        Ok(Self {
            name,
            version: u16::from_le_bytes([data[4], data[5]]),
            chip: u16::from_le_bytes([data[6], data[7]]),
            baud: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            data,
        })
    }

    pub fn from_file(path: &Path) -> AppResult<Self> {
        let data = fs::read(path)
            .map_err(|error| format!("failed to read boot loader '{}': {error}", path.display()))?;
        Self::parse(path.display().to_string(), data)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn stage1_payload(&self) -> &[u8] {
        &self.data[0x20..STAGE1_END]
    }

    pub fn stage2(&self) -> ([u8; 8], Vec<u8>) {
        let mut content = Vec::with_capacity(self.data.len());
        content.extend_from_slice(&self.data[..4]);
        content.extend_from_slice(&self.data[0x20..]);
        content.resize(self.data.len(), 0);

        let checksum = content
            .iter()
            .fold(0u16, |sum, byte| sum.wrapping_add(u16::from(*byte)));
        let mut metadata = [0u8; 8];
        metadata[..2].copy_from_slice(&checksum.to_le_bytes());
        metadata[2..4].copy_from_slice(&0x00c2u16.to_le_bytes());
        metadata[4..].copy_from_slice(&(self.data.len() as u32).to_le_bytes());
        (metadata, content)
    }
}

pub fn resolve(custom: Option<&Path>, model: Option<&str>) -> AppResult<BootImage> {
    if let Some(path) = custom {
        return BootImage::from_file(path);
    }
    let model = model.ok_or_else(|| "no boot loader was selected".to_string())?;
    embedded::load(model)
}

pub fn embedded_names() -> AppResult<Vec<String>> {
    embedded::names()
}

#[cfg(feature = "embed-loaders")]
mod embedded {
    use rust_embed::RustEmbed;

    use super::*;

    #[derive(RustEmbed)]
    #[folder = "src/loaders/"]
    #[include = "*.boot"]
    struct Loaders;

    pub fn names() -> AppResult<Vec<String>> {
        let mut names: Vec<_> = Loaders::iter().map(|name| name.into_owned()).collect();
        names.sort();
        Ok(names)
    }

    pub fn load(requested: &str) -> AppResult<BootImage> {
        let matches: Vec<_> = names()?
            .into_iter()
            .filter(|name| name == requested || name.strip_suffix(".boot") == Some(requested))
            .collect();

        let name = match matches.as_slice() {
            [name] => name,
            [] => {
                return Err(format!(
                    "embedded loader '{requested}' was not found; use --list-loaders"
                ));
            }
            _ => return Err(format!("embedded loader name '{requested}' is ambiguous")),
        };
        let file = Loaders::get(name)
            .ok_or_else(|| format!("embedded loader '{name}' disappeared from the build"))?;
        BootImage::parse(name.clone(), file.data.into_owned())
    }
}

#[cfg(not(feature = "embed-loaders"))]
mod embedded {
    use super::*;

    const ERROR: &str =
        "this build has no embedded loaders; rebuild with --features embed-loaders or use -b";

    pub fn names() -> AppResult<Vec<String>> {
        Err(ERROR.into())
    }

    pub fn load(_requested: &str) -> AppResult<BootImage> {
        Err(ERROR.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> BootImage {
        let mut data = vec![0u8; MIN_BOOT_SIZE + 17];
        data[..4].copy_from_slice(b"toob");
        data[4..6].copy_from_slice(&1u16.to_le_bytes());
        data[6..8].copy_from_slice(&0x6701u16.to_le_bytes());
        data[8..12].copy_from_slice(&115_200u32.to_le_bytes());
        for (index, byte) in data[0x20..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        BootImage::parse("test.boot", data).unwrap()
    }

    #[test]
    fn validates_header_and_builds_stage1() {
        let image = image();
        assert_eq!(image.version, 1);
        assert_eq!(image.chip, 0x6701);
        assert_eq!(image.baud, 115_200);
        assert_eq!(image.stage1_payload().len(), 8188);
    }

    #[test]
    fn stage2_skips_header_and_has_valid_metadata() {
        let image = image();
        let (metadata, content) = image.stage2();
        assert_eq!(&content[..4], b"toob");
        assert_eq!(&content[4..8], &[0, 1, 2, 3]);
        assert_eq!(content.len(), image.len());
        assert_eq!(u16::from_le_bytes([metadata[2], metadata[3]]), 0x00c2);
        assert_eq!(
            u32::from_le_bytes(metadata[4..8].try_into().unwrap()) as usize,
            image.len()
        );
        let sum = content
            .iter()
            .fold(0u16, |sum, byte| sum.wrapping_add(*byte as u16));
        assert_eq!(u16::from_le_bytes([metadata[0], metadata[1]]), sum);
    }

    #[cfg(feature = "embed-loaders")]
    #[test]
    fn embedded_loader_accepts_filename_and_stem() {
        let names = embedded_names().unwrap();
        assert!(!names.is_empty());
        let name = &names[0];
        let stem = name.strip_suffix(".boot").unwrap();
        assert_eq!(embedded::load(name).unwrap().name, *name);
        assert_eq!(embedded::load(stem).unwrap().name, *name);
    }

    #[cfg(not(feature = "embed-loaders"))]
    #[test]
    fn embedded_loader_error_explains_feature_requirement() {
        let error = embedded_names().unwrap_err();
        assert!(error.contains("--features embed-loaders"));
    }
}
