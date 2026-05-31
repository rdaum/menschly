use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::process;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use wdc65816::{HasAddressBus, Processor, StatusRegister};
use x65alike_emul::Machine;

const DEFAULT_LOAD_ADDR: u32 = 0x020000;
const DEFAULT_STEPS: usize = 1_000_000;
const STP_OPCODE: u8 = 0xdb;

fn main() {
    if let Err(error) = run() {
        eprintln!("x65alike-emul: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = Config::parse(env::args().skip(1))?;
    let image = fs::read(&config.image_path)
        .map_err(|error| format!("failed to read {}: {error}", config.image_path))?;
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

    let mut machine = Machine::new();
    machine.load(config.load_addr, &image);
    machine.push_serial_input(&serial_input);

    let mut bus = BusMachine::new(machine);
    let mut cpu = Processor::new();
    cpu.p = StatusRegister::from_byte(config.status, false);
    cpu.pbr = config.pbr;
    cpu.dbr = config.dbr;
    cpu.pc = config.pc;
    cpu.s = config.stack;

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut steps = 0usize;

    while config.max_steps == 0 || steps < config.max_steps {
        if let Some(input) = &live_input {
            drain_live_input(&mut bus.0, input);
        }
        if config
            .stop_at
            .is_some_and(|stop| cpu.pbr == stop.pbr && cpu.pc == stop.pc)
        {
            break;
        }
        if bus.read(((cpu.pbr as usize) << 16) | cpu.pc as usize) == STP_OPCODE {
            break;
        }

        cpu.step(&mut bus);
        steps += 1;
        write_serial_output(&mut bus.0, &mut stdout)?;
    }
    write_serial_output(&mut bus.0, &mut stdout)?;
    stdout
        .flush()
        .map_err(|error| format!("failed to flush stdout: {error}"))?;

    if !config.quiet {
        eprintln!(
            "x65alike-emul: steps={} pbr=${:02x} pc=${:04x} dbr=${:02x}",
            steps, cpu.pbr, cpu.pc, cpu.dbr
        );
    }

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

fn drain_live_input(machine: &mut Machine, input: &Receiver<u8>) {
    let mut bytes = Vec::new();
    while let Ok(byte) = input.try_recv() {
        bytes.push(byte);
    }
    if !bytes.is_empty() {
        machine.push_serial_input(&bytes);
    }
}

fn write_serial_output(machine: &mut Machine, stdout: &mut impl Write) -> Result<(), String> {
    let output = machine.take_serial_output();
    if !output.is_empty() {
        stdout
            .write_all(&output)
            .map_err(|error| format!("failed to write stdout: {error}"))?;
    }
    Ok(())
}

struct BusMachine(Machine);

impl BusMachine {
    fn new(machine: Machine) -> Self {
        Self(machine)
    }
}

impl HasAddressBus for BusMachine {
    fn read(&mut self, address: usize) -> u8 {
        self.0.read(address as u32)
    }

    fn write(&mut self, address: usize, value: u8) {
        self.0.write(address as u32, value);
    }

    fn io(&mut self) {}
}

#[derive(Debug, Clone)]
struct Config {
    image_path: String,
    load_addr: u32,
    pc: u16,
    pbr: u8,
    dbr: u8,
    stack: u16,
    status: u8,
    max_steps: usize,
    stop_at: Option<StopAt>,
    quiet: bool,
}

impl Config {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut image_path = None;
        let mut load_addr = DEFAULT_LOAD_ADDR;
        let mut pc = None;
        let mut pbr = None;
        let mut dbr = 0;
        let mut stack = 0x01fc;
        let mut status = 0x00;
        let mut max_steps = DEFAULT_STEPS;
        let mut stop_at = None;
        let mut quiet = false;

        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => return Err(usage()),
                "--load" => {
                    load_addr = parse_u24(next_arg(&mut args, "--load")?, "--load")?;
                }
                "--pc" => pc = Some(parse_u16(next_arg(&mut args, "--pc")?, "--pc")?),
                "--pbr" => pbr = Some(parse_u8(next_arg(&mut args, "--pbr")?, "--pbr")?),
                "--dbr" => dbr = parse_u8(next_arg(&mut args, "--dbr")?, "--dbr")?,
                "--stack" => stack = parse_u16(next_arg(&mut args, "--stack")?, "--stack")?,
                "--status" => status = parse_u8(next_arg(&mut args, "--status")?, "--status")?,
                "--max-steps" => {
                    max_steps = parse_usize(next_arg(&mut args, "--max-steps")?, "--max-steps")?
                }
                "--stop-at" => {
                    stop_at = Some(parse_stop_at(next_arg(&mut args, "--stop-at")?)?);
                }
                "--quiet" => quiet = true,
                option if option.starts_with('-') => {
                    return Err(format!("unknown option {option}\n\n{}", usage()));
                }
                path => {
                    if image_path.replace(path.to_owned()).is_some() {
                        return Err(format!("multiple image paths supplied\n\n{}", usage()));
                    }
                }
            }
        }

        let Some(image_path) = image_path else {
            return Err(usage());
        };

        Ok(Self {
            image_path,
            load_addr,
            pc: pc.unwrap_or(load_addr as u16),
            pbr: pbr.unwrap_or((load_addr >> 16) as u8),
            dbr,
            stack,
            status,
            max_steps,
            stop_at,
            quiet,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct StopAt {
    pbr: u8,
    pc: u16,
}

fn next_arg(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value\n\n{}", usage()))
}

fn parse_stop_at(value: String) -> Result<StopAt, String> {
    let Some((bank, pc)) = value.split_once(':') else {
        return Err(format!("--stop-at expects BANK:PC, got {value}"));
    };
    Ok(StopAt {
        pbr: parse_u8(bank.to_owned(), "--stop-at bank")?,
        pc: parse_u16(pc.to_owned(), "--stop-at pc")?,
    })
}

fn parse_u8(value: String, option: &str) -> Result<u8, String> {
    let value = parse_number(&value).map_err(|error| format!("{option}: {error}"))?;
    u8::try_from(value).map_err(|_| format!("{option}: value ${value:x} does not fit in u8"))
}

fn parse_u16(value: String, option: &str) -> Result<u16, String> {
    let value = parse_number(&value).map_err(|error| format!("{option}: {error}"))?;
    u16::try_from(value).map_err(|_| format!("{option}: value ${value:x} does not fit in u16"))
}

fn parse_u24(value: String, option: &str) -> Result<u32, String> {
    let value = parse_number(&value).map_err(|error| format!("{option}: {error}"))?;
    if value <= 0x00ff_ffff {
        Ok(value)
    } else {
        Err(format!("{option}: value ${value:x} does not fit in 24 bits"))
    }
}

fn parse_usize(value: String, option: &str) -> Result<usize, String> {
    let value = parse_number(&value).map_err(|error| format!("{option}: {error}"))?;
    usize::try_from(value).map_err(|_| format!("{option}: value ${value:x} is too large"))
}

fn parse_number(value: &str) -> Result<u32, String> {
    let value = value.trim();
    let (radix, digits) = value
        .strip_prefix('$')
        .map(|digits| (16, digits))
        .or_else(|| value.strip_prefix("0x").map(|digits| (16, digits)))
        .or_else(|| value.strip_prefix("0X").map(|digits| (16, digits)))
        .unwrap_or((10, value));
    u32::from_str_radix(digits, radix).map_err(|error| format!("invalid number {value}: {error}"))
}

fn usage() -> String {
    "usage: x65alike-emul [options] <image.bin>\n\
     \n\
     Options:\n\
       --load ADDR       Load image at 24-bit address, default $020000\n\
       --pc PC           Initial 16-bit PC, default low word of --load\n\
       --pbr BANK        Initial program bank, default bank of --load\n\
       --dbr BANK        Initial data bank, default $00\n\
       --stack SP        Initial stack pointer, default $01fc\n\
       --status P        Initial status register byte, default $00\n\
       --max-steps N     Maximum CPU steps, default 1000000; 0 runs indefinitely\n\
       --stop-at B:PC    Stop before executing when PBR:PC matches\n\
       --quiet           Suppress final CPU status on stderr\n\
     \n\
     Stdin is queued as serial RX. Writes to SERIAL_DATA ($9f70) are streamed to stdout."
        .to_owned()
}
