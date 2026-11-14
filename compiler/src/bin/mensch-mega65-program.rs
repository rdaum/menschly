//! Builds a Hellord program for the MEGA65: an o65 file that `RUN` loads.
//!
//! ```text
//! mensch-mega65-program -o HELLO.PRG hello.mensch [more.mensch...]
//! ```

use std::fs;
use std::path::PathBuf;
use std::process;

use clap::Parser;
use mensch_systems_tests::mega65;

#[derive(Debug, Parser)]
#[command(about = "Build a relocatable Hellord program (o65) for the MEGA65")]
struct Config {
    /// Mensch source modules; one must export `function main(): u8`.
    #[arg(required = true)]
    sources: Vec<PathBuf>,

    /// Output file (use an 8.3 name ending in .PRG to RUN it by name).
    #[arg(short, long)]
    output: PathBuf,

    /// Directory for intermediate files (default: a fresh temp directory).
    #[arg(long)]
    work_dir: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run(Config::parse()) {
        eprintln!("mensch-mega65-program: {error}");
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
    let modules = mega65::compile_program_modules(&sources)?;
    let dir = config.work_dir.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("mensch-mega65-program-{}", process::id()))
    });
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let program = mega65::build_program(&dir, &refs)?;
    fs::write(&config.output, &program)
        .map_err(|error| format!("write {}: {error}", config.output.display()))?;
    eprintln!("{}: {} bytes", config.output.display(), program.len());
    Ok(())
}
