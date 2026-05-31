use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use clap::Parser;
use mensch_systems_tests::{ca65, frontend, lower_65816, storage};

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
        let mut assignments = Vec::new();
        for source_path in &config.sources {
            if source_path == Path::new("-") {
                return Err("stdin cannot be used with multiple input paths".to_owned());
            }
            let compiled = compile_path(source_path, config.storage_report)?;
            let stem = source_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("source");
            let output_path = output_dir.join(format!("{stem}.a65"));
            fs::write(&output_path, compiled.asm)
                .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;
            assignments.push(compiled.assignment);
        }
        if config.storage_report {
            eprintln!(
                "{}",
                storage::direct_page_usage(&assignments, storage::DEFAULT_DIRECT_PAGE_BUDGET)
                    .report()
            );
        }
        return Ok(());
    }

    let compiled = compile_path(&config.sources[0], config.storage_report)?;

    if let Some(output) = config.output {
        fs::write(&output, compiled.asm)
            .map_err(|error| format!("failed to write {}: {error}", output.display()))?;
    } else {
        io::stdout()
            .write_all(compiled.asm.as_bytes())
            .map_err(|error| format!("failed to write stdout: {error}"))?;
    }

    Ok(())
}

struct CompiledSource {
    asm: String,
    assignment: storage::StorageAssignment,
}

fn compile_path(path: &Path, storage_report: bool) -> Result<CompiledSource, String> {
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
    let (lowered, assignment) =
        lower_65816::lower_module_with_storage(&module).map_err(|error| error.to_string())?;
    if storage_report {
        if path == Path::new("-") {
            eprintln!("{assignment}");
        } else {
            eprintln!("{}\n{assignment}", path.display());
        }
    }
    Ok(CompiledSource {
        asm: ca65::emit_module(&lowered),
        assignment,
    })
}

#[derive(Debug, Parser)]
#[command(about = "Compile Mensch source to ca65 assembly")]
struct Config {
    /// Mensch source files. Reads stdin when omitted.
    sources: Vec<PathBuf>,

    /// Write ca65 assembly to a file, or to a directory when compiling multiple sources.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Print the compiler's storage and direct-page assignment report to stderr.
    #[arg(long)]
    storage_report: bool,
}
