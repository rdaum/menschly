//! Compiles Mensch modules into a MEGA65 kernel ROM and boots it in Xemu.
//!
//! ```text
//! mensch-run-mega65 [--type-in TEXT] [--rom OUT.rom] [--no-run]
//!     [--window N:module.mensch]... module.mensch...
//! ```
//!
//! Modules given with `--window` have their code placed in window bank `N`
//! (see `mega65`); the others are resident.

use std::fs;
use std::path::PathBuf;
use std::process;
use std::time::Duration;

use clap::Parser;
use menschly::mega65::{self, XemuOptions};

#[derive(Debug, Parser)]
#[command(about = "Build a MEGA65 kernel ROM from Mensch modules and boot it in Xemu")]
struct Config {
    /// Mensch source modules; one must export `main`.
    #[arg(required = true)]
    sources: Vec<PathBuf>,

    /// A module whose code goes in window bank N (1-4), as `N:PATH`
    /// (repeatable).
    #[arg(long = "window", value_name = "N:PATH")]
    windows: Vec<String>,

    /// Text typed into the keyboard queue as the kernel reads it (`\n` is RETURN).
    #[arg(long = "type-in")]
    type_in: Option<String>,

    /// Put this host file in the root directory of the virtual SD card
    /// (repeatable; use 8.3 file names).
    #[arg(long = "sd-file")]
    sd_files: Vec<PathBuf>,

    /// Also write the 128 KiB ROM image here (usable as MEGA65.ROM).
    #[arg(long)]
    rom: Option<PathBuf>,

    /// Build only; do not start Xemu.
    #[arg(long)]
    no_run: bool,

    /// Seconds to run before stopping Xemu (the kernel may exit sooner via $D6CF).
    #[arg(long, default_value_t = 5)]
    timeout: u64,

    /// Directory for intermediate files (default: a fresh temp directory).
    #[arg(long)]
    work_dir: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run(Config::parse()) {
        eprintln!("mensch-run-mega65: {error}");
        process::exit(1);
    }
}

fn run(config: Config) -> Result<(), String> {
    let mut placed = config
        .sources
        .iter()
        .map(|path| (path.clone(), None))
        .collect::<Vec<_>>();
    for spec in &config.windows {
        let (bank, path) = spec
            .split_once(':')
            .ok_or_else(|| format!("--window {spec}: expected N:PATH"))?;
        let bank = bank
            .parse::<u8>()
            .map_err(|_| format!("--window {spec}: bad bank number"))?;
        placed.push((PathBuf::from(path), Some(bank)));
    }
    let mut sources = Vec::new();
    for (path, window) in placed {
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("module")
            .to_owned();
        sources.push(mega65::KernelModule {
            name: format!("{}-{stem}", sources.len()),
            source,
            window,
        });
    }
    let modules = mega65::compile_kernel_modules(&sources)?;
    let dir = config.work_dir.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("mensch-mega65-run-{}", process::id()))
    });
    let module_refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let rom = mega65::build_rom(&dir, &module_refs)?;
    if let Some(path) = &config.rom {
        fs::write(path, &rom).map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    if config.no_run {
        return Ok(());
    }
    let xemu = mega65::find_xemu().ok_or("Xemu not found (set MENSCH_XMEGA65)")?;
    let mut sd_files = Vec::new();
    for path in &config.sd_files {
        let contents = fs::read(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("bad file name: {}", path.display()))?
            .to_owned();
        sd_files.push((name, contents));
    }
    let run = mega65::run_rom_with(
        &xemu,
        &dir,
        &rom,
        &XemuOptions {
            timeout: Duration::from_secs(config.timeout),
            type_in: config.type_in,
            stop_at_timeout: true,
            sd_files,
        },
    )?;
    print!("{}", run.screen);
    if run.timed_out {
        eprintln!("(stopped after {} s)", config.timeout);
    } else if run.exit_code != 0 {
        eprintln!("kernel exit status {}", run.exit_code);
    }
    Ok(())
}
