use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::process;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use clap::Parser;
use mensch_systems_tests::x65_runner::{
    build_mensch_xex_sources, run_image, BlockDeviceConfig, RunConfig,
};

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
    let build = build_mensch_xex_sources(build_name, &config.sources, &config.linker_config)?;

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
    about = "Compile a Mensch source file, link it as an X65-style XEX, and boot it in x65alike-emul"
)]
struct Config {
    /// Mensch source files to compile and boot.
    #[arg(required = true)]
    sources: Vec<PathBuf>,

    /// ld65 linker config used to produce an Atari/XEX image.
    #[arg(short = 'C', long = "config")]
    linker_config: PathBuf,

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
