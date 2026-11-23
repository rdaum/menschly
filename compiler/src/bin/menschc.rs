use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use clap::Parser;
use menschly::driver::{self, Target};
use menschly::{frontend, storage};

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
    let mut texts = Vec::new();
    for path in &config.sources {
        texts.push(read_source(path)?);
    }
    let modules = driver::import_sources(&texts, &config.import_paths)?;

    if config.sources.len() > 1 {
        let Some(output_dir) = &config.output else {
            return Err("multiple input paths require --output DIR".to_owned());
        };
        fs::create_dir_all(output_dir)
            .map_err(|error| format!("failed to create {}: {error}", output_dir.display()))?;
        let mut assignments = Vec::new();
        for (source_path, source) in config.sources.iter().zip(&texts) {
            if source_path == Path::new("-") {
                return Err("stdin cannot be used with multiple input paths".to_owned());
            }
            let remaining_direct_page = config
                .direct_page_budget
                .saturating_sub(assignments_direct_page_bytes(&assignments));
            let compiled = compile_text(
                source_path,
                source,
                &modules,
                config.target,
                config.storage_report,
                remaining_direct_page,
            )?;
            let stem = source_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("source");
            let output_path = output_dir.join(format!("{stem}.a65"));
            fs::write(&output_path, compiled.asm)
                .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;
            assignments.push(compiled.assignment);
        }
        storage::validate_direct_page_budget(&assignments, config.direct_page_budget)
            .map_err(|error| error.to_string())?;
        if config.storage_report {
            eprintln!(
                "{}",
                storage::direct_page_usage(&assignments, config.direct_page_budget).report()
            );
        }
        return Ok(());
    }

    let compiled = compile_text(
        &config.sources[0],
        &texts[0],
        &modules,
        config.target,
        config.storage_report,
        config.direct_page_budget,
    )?;
    storage::validate_direct_page_budget(
        std::slice::from_ref(&compiled.assignment),
        config.direct_page_budget,
    )
    .map_err(|error| error.to_string())?;

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

fn read_source(path: &Path) -> Result<String, String> {
    if path != Path::new("-") {
        return fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()));
    }
    let mut source = String::new();
    io::stdin()
        .read_to_string(&mut source)
        .map_err(|error| format!("failed to read stdin: {error}"))?;
    Ok(source)
}

fn compile_text(
    path: &Path,
    source: &str,
    modules: &dyn frontend::ModuleSource,
    target: Target,
    storage_report: bool,
    direct_page_budget: u16,
) -> Result<CompiledSource, String> {
    let compiled = driver::compile_source_with(source, modules, target, direct_page_budget)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let assignment = compiled.assignment;
    if storage_report {
        if path == Path::new("-") {
            eprintln!("{assignment}");
        } else {
            eprintln!("{}\n{assignment}", path.display());
        }
    }
    Ok(CompiledSource {
        asm: compiled.asm,
        assignment,
    })
}

fn assignments_direct_page_bytes(assignments: &[storage::StorageAssignment]) -> u16 {
    assignments
        .iter()
        .map(|assignment| assignment.direct_page_bytes)
        .sum()
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

    /// Direct-page bytes available to compiler-managed storage.
    #[arg(long, default_value_t = storage::DEFAULT_DIRECT_PAGE_BUDGET)]
    direct_page_budget: u16,

    /// A directory of modules to import from (repeatable); the sources
    /// given are always importable.
    #[arg(short = 'I', long = "import-path", value_name = "DIR")]
    import_paths: Vec<PathBuf>,

    /// Target CPU/machine: `65816` (x65-alike) or `mega65` (45GS02).
    #[arg(long, default_value = "65816")]
    target: Target,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn compile_path_spills_scratch_when_direct_page_budget_is_full() {
        let path = temp_source_path("scratch-budget");
        fs::write(
            &path,
            "
            module ScratchBudget

            export procedure main()
                direct var reserved: u16
                scratch var value: u16

                value = $1234
                reserved = value
                return
            end
            ",
        )
        .expect("source fixture should be written");

        let source = read_source(&path).expect("source should be readable");
        let compiled = compile_text(
            &path,
            &source,
            &frontend::NoModules,
            Target::W65816,
            false,
            2,
        )
        .expect("source should compile");

        assert_eq!(compiled.assignment.direct_page_bytes, 2);
        assert!(compiled.asm.contains("$01,s"));
        assert!(!compiled.asm.contains("__dp_scratch"));
    }

    fn temp_source_path(name: &str) -> PathBuf {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("menschc-{name}-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir.join("fixture.mensch")
    }
}
