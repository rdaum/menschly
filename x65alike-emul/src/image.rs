use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Auto,
    Raw,
    AtariXex,
}

impl ImageFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "raw" | "bin" => Ok(Self::Raw),
            "xex" | "atari" | "atari-xex" => Ok(Self::AtariXex),
            _ => Err(format!(
                "unknown image format {value}; expected auto, raw, or xex"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuStart {
    pub pbr: u8,
    pub pc: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedRange {
    pub address: u32,
    pub bytes: Vec<u8>,
    pub cpu_visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedImage {
    pub ranges: Vec<LoadedRange>,
    pub entry: Option<CpuStart>,
}

pub fn load_image(
    path: &Path,
    bytes: &[u8],
    format: ImageFormat,
    raw_load_addr: u32,
) -> Result<LoadedImage, String> {
    match resolve_format(path, format) {
        ImageFormat::Auto => unreachable!("auto should resolve to a concrete image format"),
        ImageFormat::Raw => Ok(LoadedImage {
            ranges: vec![LoadedRange {
                address: raw_load_addr,
                bytes: bytes.to_vec(),
                cpu_visible: false,
            }],
            entry: Some(CpuStart {
                pbr: (raw_load_addr >> 16) as u8,
                pc: raw_load_addr as u16,
            }),
        }),
        ImageFormat::AtariXex => parse_atari_xex(bytes),
    }
}

fn resolve_format(path: &Path, format: ImageFormat) -> ImageFormat {
    if format != ImageFormat::Auto {
        return format;
    }

    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("xex") || ext.eq_ignore_ascii_case("obx") => {
            ImageFormat::AtariXex
        }
        Some(ext) if ext.eq_ignore_ascii_case("bin") || ext.eq_ignore_ascii_case("raw") => {
            ImageFormat::Raw
        }
        _ => ImageFormat::Raw,
    }
}

fn parse_atari_xex(bytes: &[u8]) -> Result<LoadedImage, String> {
    let mut pos = 0usize;
    let mut ranges = Vec::new();
    let mut entry = None;

    while pos < bytes.len() {
        let start = read_u16(bytes, &mut pos, "Atari/XEX block start")?;
        if start == 0xffff {
            continue;
        }

        let end = read_u16(bytes, &mut pos, "Atari/XEX block end")?;
        if end < start {
            return Err(format!(
                "Atari/XEX block end ${end:04x} is before start ${start:04x}"
            ));
        }

        let len = usize::from(end - start) + 1;
        let block_end = pos
            .checked_add(len)
            .ok_or_else(|| "Atari/XEX block length overflowed".to_owned())?;
        if block_end > bytes.len() {
            return Err(format!(
                "Atari/XEX block ${start:04x}-${end:04x} extends past end of file"
            ));
        }

        let data = bytes[pos..block_end].to_vec();
        if let Some(runad) = runad_in_block(start, &data) {
            entry = Some(CpuStart { pbr: 0, pc: runad });
        }
        ranges.push(LoadedRange {
            address: u32::from(start),
            bytes: data,
            cpu_visible: true,
        });
        pos = block_end;
    }

    if ranges.is_empty() {
        return Err("Atari/XEX image contains no load blocks".to_owned());
    }

    Ok(LoadedImage { ranges, entry })
}

fn read_u16(bytes: &[u8], pos: &mut usize, context: &str) -> Result<u16, String> {
    let end = pos
        .checked_add(2)
        .ok_or_else(|| format!("{context} offset overflowed"))?;
    if end > bytes.len() {
        return Err(format!("truncated {context}"));
    }
    let value = u16::from_le_bytes([bytes[*pos], bytes[*pos + 1]]);
    *pos = end;
    Ok(value)
}

fn runad_in_block(start: u16, data: &[u8]) -> Option<u16> {
    const RUNAD: u16 = 0x02e0;
    let runad_offset = RUNAD.checked_sub(start)? as usize;
    let high_offset = runad_offset.checked_add(1)?;
    if high_offset < data.len() {
        Some(u16::from_le_bytes([data[runad_offset], data[high_offset]]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_atari_xex_blocks_and_runad() {
        let bytes = [
            0xff, 0xff, 0x00, 0x80, 0x02, 0x80, 0xa9, 0x21, 0xdb, 0xe0, 0x02, 0xe1, 0x02, 0x00,
            0x80,
        ];

        let image = parse_atari_xex(&bytes).unwrap();

        assert_eq!(
            image.entry,
            Some(CpuStart {
                pbr: 0,
                pc: 0x8000
            })
        );
        assert_eq!(
            image.ranges,
            vec![
                LoadedRange {
                    address: 0x8000,
                    bytes: vec![0xa9, 0x21, 0xdb],
                    cpu_visible: true,
                },
                LoadedRange {
                    address: 0x02e0,
                    bytes: vec![0x00, 0x80],
                    cpu_visible: true,
                },
            ]
        );
    }

    #[test]
    fn rejects_truncated_atari_xex_block() {
        let error =
            parse_atari_xex(&[0xff, 0xff, 0x00, 0x80, 0x02, 0x80, 0xa9]).unwrap_err();

        assert!(error.contains("extends past end of file"));
    }

    #[test]
    fn auto_detects_xex_by_extension_before_raw_fallback() {
        let image = load_image(
            Path::new("program.xex"),
            &[0xff, 0xff, 0x00, 0x80, 0x00, 0x80, 0xdb],
            ImageFormat::Auto,
            0x020000,
        )
        .unwrap();
        assert_eq!(image.ranges[0].address, 0x8000);

        let image = load_image(
            Path::new("program.bin"),
            &[0xff, 0xff, 0x00, 0x80, 0x00, 0x80, 0xdb],
            ImageFormat::Auto,
            0x020000,
        )
        .unwrap();
        assert_eq!(image.ranges[0].address, 0x020000);

        let image = load_image(
            Path::new("program"),
            &[0xff, 0xff, 0x00, 0x80, 0x00, 0x80, 0xdb],
            ImageFormat::Auto,
            0x020000,
        )
        .unwrap();
        assert_eq!(image.ranges[0].address, 0x020000);
    }
}
