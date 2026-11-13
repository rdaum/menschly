//! Compiles Mensch modules into a MEGA65 kernel ROM and boots it in Xemu.
//!
//! ```text
//! mensch-run-mega65 [--type-in TEXT] [--rom OUT.rom] [--no-run] module.mensch...
//! ```

use std::fs;
use std::path::PathBuf;
use std::process;
use std::time::Duration;

use clap::Parser;
use mensch_systems_tests::mega65::{self, XemuOptions};

#[derive(Debug, Parser)]
#[command(about = "Build a MEGA65 kernel ROM from Mensch modules and boot it in Xemu")]
struct Config {
    /// Mensch source modules; one must export `main`.
    #[arg(required = true)]
    sources: Vec<PathBuf>,

    /// Text typed into the keyboard queue as the kernel reads it (`\n` is RETURN).
    #[arg(long = "type-in")]
    type_in: Option<String>,

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
    let mut sources = Vec::new();
    for path in &config.sources {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("module")
            .to_owned();
        sources.push((format!("{}-{stem}", sources.len()), source));
    }
    let modules = mega65::compile_modules(&sources)?;
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
    let run = mega65::run_rom_with(
        &xemu,
        &dir,
        &rom,
        &XemuOptions {
            timeout: Duration::from_secs(config.timeout),
            type_in: config.type_in,
            stop_at_timeout: true,
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
