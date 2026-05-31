use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use clap::Parser;
use mensch_systems_tests::{ca65, frontend, lower_65816};

fn main() {
    if let Err(error) = run() {
        eprintln!("menschc: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut config = Config::parse();
    if config.sources.is_empty() {
        config.sources.push(PathBuf::from("-"));
    }

    if config.sources.len() > 1 {
        let Some(output_dir) = &config.output else {
            return Err("multiple input paths require --output DIR".to_owned());
        };
        fs::create_dir_all(output_dir)
            .map_err(|error| format!("failed to create {}: {error}", output_dir.display()))?;
        for source_path in &config.sources {
            if source_path == Path::new("-") {
                return Err("stdin cannot be used with multiple input paths".to_owned());
            }
            let asm = compile_path(source_path)?;
            let stem = source_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("source");
            let output_path = output_dir.join(format!("{stem}.a65"));
            fs::write(&output_path, asm)
                .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;
        }
        return Ok(());
    }

    let asm = compile_path(&config.sources[0])?;

    if let Some(output) = config.output {
        fs::write(&output, asm)
            .map_err(|error| format!("failed to write {}: {error}", output.display()))?;
    } else {
        io::stdout()
            .write_all(asm.as_bytes())
            .map_err(|error| format!("failed to write stdout: {error}"))?;
    }

    Ok(())
}

fn compile_path(path: &Path) -> Result<String, String> {
    let mut source = String::new();
    if path == Path::new("-") {
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
    } else {
        source = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    }

    let module = frontend::compile_source(&source).map_err(|error| error.to_string())?;
    let (lowered, _assignment) =
        lower_65816::lower_module_with_storage(&module).map_err(|error| error.to_string())?;
    Ok(ca65::emit_module(&lowered))
}

#[derive(Debug, Parser)]
#[command(about = "Compile Mensch source to ca65 assembly")]
struct Config {
    /// Mensch source files. Reads stdin when omitted.
    sources: Vec<PathBuf>,

    /// Write ca65 assembly to a file, or to a directory when compiling multiple sources.
    #[arg(short, long)]
    output: Option<PathBuf>,
}
