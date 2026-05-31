use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{SystemTime, UNIX_EPOCH};

use wdc65816::{HasAddressBus, Processor, StatusRegister};
use x65alike_emul::{
    image::{load_image, ImageFormat, LoadedImage},
    Machine,
};

use crate::{ca65, frontend, lower_65816};

const DEFAULT_RAW_LOAD_ADDR: u32 = 0x020000;
const NATIVE_IRQ_VECTOR: usize = 0x00ffee;
const EMULATION_IRQ_VECTOR: usize = 0x00fffe;
const STP_OPCODE: u8 = 0xdb;

static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

pub struct BuildOutput {
    pub image: LoadedImage,
}

pub struct RunResult {
    #[allow(dead_code)]
    pub cpu: Processor,
    pub machine: Machine,
    pub output: Vec<u8>,
}

pub struct RunConfig {
    pub serial_input: Vec<u8>,
    pub max_steps: usize,
    pub stop_after_output_len: Option<usize>,
    pub live_input: Option<Receiver<u8>>,
}

impl RunConfig {
    pub fn scripted(serial_input: &[u8], max_steps: usize) -> Self {
        Self {
            serial_input: serial_input.to_vec(),
            max_steps,
            stop_after_output_len: None,
            live_input: None,
        }
    }
}

pub fn build_mensch_xex(
    name: &str,
    source_path: impl AsRef<Path>,
    cfg_path: impl AsRef<Path>,
) -> Result<BuildOutput, String> {
    build_mensch_xex_sources(name, &[source_path.as_ref().to_path_buf()], cfg_path)
}

pub fn build_mensch_xex_sources(
    name: &str,
    source_paths: &[PathBuf],
    cfg_path: impl AsRef<Path>,
) -> Result<BuildOutput, String> {
    if source_paths.is_empty() {
        return Err("at least one Mensch source path is required".to_owned());
    }

    let dir = temp_dir(name);
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create build dir: {error}"))?;
    let xex_path = dir.join(format!("{name}.xex"));
    let mut obj_paths = Vec::new();

    for (index, source_path) in source_paths.iter().enumerate() {
        let stem = source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("source");
        let asm_path = dir.join(format!("{index}-{stem}.a65"));
        let obj_path = dir.join(format!("{index}-{stem}.o"));

        let source = fs::read_to_string(source_path)
            .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?;
        let module = frontend::compile_source(&source).map_err(|error| error.to_string())?;
        let (lowered, _assignment) =
            lower_65816::lower_module_with_storage(&module).map_err(|error| error.to_string())?;
        fs::write(&asm_path, ca65::emit_module(&lowered))
            .map_err(|error| format!("failed to write {}: {error}", asm_path.display()))?;

        run_command(
            Command::new("ca65")
                .arg("-g")
                .arg("--cpu")
                .arg("65816")
                .arg(&asm_path)
                .arg("-o")
                .arg(&obj_path),
        )?;
        obj_paths.push(obj_path);
    }

    let mut ld65 = Command::new("ld65");
    ld65.arg("-C").arg(cfg_path.as_ref());
    for obj_path in &obj_paths {
        ld65.arg(obj_path);
    }
    ld65.arg("-o").arg(&xex_path);
    run_command(&mut ld65)?;

    let xex = fs::read(&xex_path)
        .map_err(|error| format!("failed to read {}: {error}", xex_path.display()))?;
    let image = load_image(&xex_path, &xex, ImageFormat::Auto, DEFAULT_RAW_LOAD_ADDR)?;

    Ok(BuildOutput { image })
}

pub fn machine_with_loaded_image(image: &LoadedImage) -> Machine {
    let mut machine = Machine::new();
    for range in &image.ranges {
        if range.cpu_visible {
            machine.load_cpu_visible(range.address, &range.bytes);
        } else {
            machine.load(range.address, &range.bytes);
        }
    }
    machine
}

pub fn run_image_until_output(
    image: &LoadedImage,
    serial_input: &[u8],
    output_len: usize,
    max_steps: usize,
) -> Result<RunResult, String> {
    let mut config = RunConfig::scripted(serial_input, max_steps);
    config.stop_after_output_len = Some(output_len);
    let mut output = Vec::new();
    let result = run_image(image, config, &mut output)?;
    if output.len() < output_len {
        return Err(format!(
            "image produced {} output bytes, expected at least {output_len}",
            output.len()
        ));
    }
    Ok(result)
}

pub fn run_image(
    image: &LoadedImage,
    config: RunConfig,
    output: &mut impl Write,
) -> Result<RunResult, String> {
    let mut machine = machine_with_loaded_image(image);
    machine.push_serial_input(&config.serial_input);

    let entry = image
        .entry
        .ok_or_else(|| "image should include an entry point".to_owned())?;
    let mut bus = BusMachine::new(machine);
    let mut cpu = Processor::new();
    cpu.p = StatusRegister::from_byte(0x00, false);
    cpu.pbr = entry.pbr;
    cpu.dbr = 0;
    cpu.pc = entry.pc;
    cpu.s = 0x01fc;

    let mut steps = 0usize;
    let mut captured_output = Vec::new();
    while config.max_steps == 0 || steps < config.max_steps {
        if let Some(input) = &config.live_input {
            drain_live_input(&mut bus.0, input);
        }
        if bus.read(((cpu.pbr as usize) << 16) | cpu.pc as usize) == STP_OPCODE {
            break;
        }
        if !cpu.p.i && bus.0.irq_pending() {
            service_irq(&mut cpu, &mut bus);
        }
        cpu.step(&mut bus);
        steps += 1;
        drain_serial_output(&mut bus.0, output, &mut captured_output)?;
        if config
            .stop_after_output_len
            .is_some_and(|len| captured_output.len() >= len)
        {
            break;
        }
    }
    drain_serial_output(&mut bus.0, output, &mut captured_output)?;
    output
        .flush()
        .map_err(|error| format!("failed to flush output: {error}"))?;

    Ok(RunResult {
        cpu,
        machine: bus.into_inner(),
        output: captured_output,
    })
}

pub fn run_command(command: &mut Command) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("failed to execute command {command:?}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "command failed: {command:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mensch-x65-{name}-{}-{nanos}-{id}",
        std::process::id()
    ))
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

fn drain_serial_output(
    machine: &mut Machine,
    output: &mut impl Write,
    captured_output: &mut Vec<u8>,
) -> Result<(), String> {
    let bytes = machine.take_serial_output();
    if !bytes.is_empty() {
        captured_output.extend_from_slice(&bytes);
        output
            .write_all(&bytes)
            .map_err(|error| format!("failed to write output: {error}"))?;
    }
    Ok(())
}

struct BusMachine(Machine);

impl BusMachine {
    fn new(machine: Machine) -> Self {
        Self(machine)
    }

    fn into_inner(self) -> Machine {
        self.0
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

fn service_irq(cpu: &mut Processor, machine: &mut BusMachine) {
    if cpu.p.e {
        push_u16(cpu, machine, cpu.pc);
        let mut p = cpu.p;
        p.xb = false;
        push_u8(cpu, machine, p.to_byte(true));
        cpu.pbr = 0;
        cpu.pc = read_u16(machine, EMULATION_IRQ_VECTOR);
    } else {
        push_u8(cpu, machine, cpu.pbr);
        push_u16(cpu, machine, cpu.pc);
        push_u8(cpu, machine, cpu.p.to_byte(true));
        cpu.pbr = 0;
        cpu.pc = read_u16(machine, NATIVE_IRQ_VECTOR);
    }
    cpu.p.d = false;
    cpu.p.i = true;
}

fn push_u8(cpu: &mut Processor, machine: &mut BusMachine, value: u8) {
    machine.write(cpu.s as usize, value);
    cpu.s = cpu.s.wrapping_sub(1);
    if cpu.p.e {
        cpu.s = 0x0100 | (cpu.s & 0x00ff);
    }
}

fn push_u16(cpu: &mut Processor, machine: &mut BusMachine, value: u16) {
    let [low, high] = value.to_le_bytes();
    push_u8(cpu, machine, high);
    push_u8(cpu, machine, low);
}

fn read_u16(machine: &mut BusMachine, address: usize) -> u16 {
    u16::from_le_bytes([machine.read(address), machine.read(address + 1)])
}
