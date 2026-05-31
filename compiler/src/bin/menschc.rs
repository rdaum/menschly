use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process;

use mensch_systems_tests::{ca65, frontend, lower_65816};

fn main() {
    if let Err(error) = run() {
        eprintln!("menschc: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = Config::parse(env::args().skip(1))?;
    let mut source = String::new();
    if config.input == "-" {
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
    } else {
        source = fs::read_to_string(&config.input)
            .map_err(|error| format!("failed to read {}: {error}", config.input))?;
    }

    let module = frontend::compile_source(&source).map_err(|error| error.to_string())?;
    let (lowered, _assignment) =
        lower_65816::lower_module_with_storage(&module).map_err(|error| error.to_string())?;
    let asm = ca65::emit_module(&lowered);

    if let Some(output) = config.output {
        fs::write(&output, asm).map_err(|error| format!("failed to write {output}: {error}"))?;
    } else {
        io::stdout()
            .write_all(asm.as_bytes())
            .map_err(|error| format!("failed to write stdout: {error}"))?;
    }

    Ok(())
}

struct Config {
    input: String,
    output: Option<String>,
}

impl Config {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut input = None;
        let mut output = None;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => return Err(usage()),
                "-o" | "--output" => {
                    let value = args
                        .next()
                        .ok_or_else(|| format!("{arg} requires a value\n\n{}", usage()))?;
                    output = Some(value);
                }
                option if option.starts_with('-') && option != "-" => {
                    return Err(format!("unknown option {option}\n\n{}", usage()));
                }
                path => {
                    if input.replace(path.to_owned()).is_some() {
                        return Err(format!("multiple input paths supplied\n\n{}", usage()));
                    }
                }
            }
        }

        Ok(Self {
            input: input.unwrap_or_else(|| "-".to_owned()),
            output,
        })
    }
}

fn usage() -> String {
    "usage: menschc [options] [source.mensch]\n\
     \n\
     Options:\n\
       -o, --output PATH    Write ca65 assembly to PATH instead of stdout\n\
     \n\
     Reads source from stdin when no source path is supplied."
        .to_owned()
}
