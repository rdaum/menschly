use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::process;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use clap::{Parser, ValueEnum};
use mensch_systems_tests::x65_runner::{
    build_mensch_x65_image_sources, build_mensch_xex_sources, run_image, BlockDeviceConfig,
    RunConfig, X65ImageBlock,
};
use x65alike_emul::image::CpuStart;

const DEFAULT_STEPS: usize = 1_000_000;

fn main() {
    if let Err(error) = run() {
        eprintln!("mensch-run-x65: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = Config::parse();
    let build_name = config
        .sources
        .first()
        .and_then(|source| source.file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("mensch-image");
    let build = match config.format {
        BuildFormat::Xex => {
            build_mensch_xex_sources(build_name, &config.sources, &config.linker_config)?
        }
        BuildFormat::X65 => build_mensch_x65_image_sources(
            build_name,
            &config.sources,
            &config.linker_config,
            CpuStart {
                pbr: config.entry_bank,
                pc: config.entry,
            },
            &config
                .blocks
                .iter()
                .map(|block| X65ImageBlock {
                    address: block.address,
                    path: block.path.clone(),
                })
                .collect::<Vec<_>>(),
        )?,
    };

    let mut serial_input = Vec::new();
    let mut stdin = io::stdin();
    let live_input = if stdin.is_terminal() {
        Some(spawn_stdin_reader())
    } else {
        stdin
            .read_to_end(&mut serial_input)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
        None
    };

    let run_config = RunConfig {
        serial_input,
        max_steps: config.max_steps,
        stop_after_output_len: None,
        live_input,
        block_device: read_block_device_config(&config)?,
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let result = run_image(&build.image, run_config, &mut stdout)?;
    if let Some(disk_path) = &config.disk {
        if !config.disk_read_only {
            if let Some(image) = result.machine.block_device_image() {
                fs::write(disk_path, image).map_err(|error| {
                    format!(
                        "failed to write disk image {}: {error}",
                        disk_path.display()
                    )
                })?;
            }
        }
    }

    Ok(())
}

fn read_block_device_config(config: &Config) -> Result<Option<BlockDeviceConfig>, String> {
    let Some(disk_path) = &config.disk else {
        return Ok(None);
    };
    let image = fs::read(disk_path)
        .map_err(|error| format!("failed to read disk image {}: {error}", disk_path.display()))?;
    Ok(Some(BlockDeviceConfig {
        image,
        read_only: config.disk_read_only,
    }))
}

fn spawn_stdin_reader() -> Receiver<u8> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        for byte in stdin.lock().bytes() {
            let Ok(byte) = byte else {
                break;
            };
            if sender.send(byte).is_err() {
                break;
            }
        }
    });
    receiver
}

#[derive(Debug, Clone, Parser)]
#[command(
    about = "Compile Mensch source files, link an x65-alike image, and boot it in x65alike-emul"
)]
struct Config {
    /// Mensch source files to compile and boot.
    #[arg(required = true)]
    sources: Vec<PathBuf>,

    /// ld65 linker config used to produce an Atari/XEX image.
    #[arg(short = 'C', long = "config")]
    linker_config: PathBuf,

    /// Linked image format to boot.
    #[arg(long = "format", value_enum, default_value_t = BuildFormat::Xex)]
    format: BuildFormat,

    /// Entry program bank for --format x65.
    #[arg(long = "entry-bank", value_parser = parse_u8_hex, default_value = "0")]
    entry_bank: u8,

    /// Entry program counter for --format x65.
    #[arg(long = "entry", value_parser = parse_u16_hex, default_value = "a000")]
    entry: u16,

    /// x65 load block as address:path. Repeat for each linked output file.
    #[arg(long = "block", value_parser = parse_x65_block)]
    blocks: Vec<CliX65Block>,

    /// Maximum CPU steps. Use 0 to run indefinitely.
    #[arg(long = "max-steps", default_value_t = DEFAULT_STEPS)]
    max_steps: usize,

    /// Attach a 512-byte-sector block disk image at the x65-alike block device.
    #[arg(long = "disk")]
    disk: Option<PathBuf>,

    /// Attach --disk without writing changes back on exit.
    #[arg(long = "disk-read-only")]
    disk_read_only: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BuildFormat {
    Xex,
    X65,
}

impl std::fmt::Display for BuildFormat {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Xex => out.write_str("xex"),
            Self::X65 => out.write_str("x65"),
        }
    }
}

#[derive(Debug, Clone)]
struct CliX65Block {
    address: u32,
    path: PathBuf,
}

fn parse_x65_block(value: &str) -> Result<CliX65Block, String> {
    let (address, path) = value
        .split_once(':')
        .ok_or_else(|| "x65 block must be address:path".to_owned())?;
    let address = parse_u24(address)?;
    Ok(CliX65Block {
        address,
        path: path.into(),
    })
}

fn parse_u24(value: &str) -> Result<u32, String> {
    let trimmed = value
        .strip_prefix('$')
        .or_else(|| value.strip_prefix("0x"))
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let address = u32::from_str_radix(trimmed, 16)
        .map_err(|error| format!("invalid x65 block address {value}: {error}"))?;
    if address > 0x00ff_ffff {
        return Err(format!("x65 block address {value} exceeds 24 bits"));
    }
    Ok(address)
}

fn parse_u16_hex(value: &str) -> Result<u16, String> {
    let value = parse_u24(value)?;
    u16::try_from(value).map_err(|_| "entry must fit in 16 bits".to_owned())
}

fn parse_u8_hex(value: &str) -> Result<u8, String> {
    let value = parse_u24(value)?;
    u8::try_from(value).map_err(|_| "entry bank must fit in 8 bits".to_owned())
}
