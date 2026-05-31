use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;
use std::process;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use clap::Parser;
use mensch_systems_tests::x65_runner::{build_mensch_xex_sources, run_image, RunConfig};

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
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    run_image(&build.image, run_config, &mut stdout)?;

    Ok(())
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
}
