use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mensch_systems_tests::{ca65, frontend, lower_65816};
use wdc65816::{HasAddressBus, Processor, StatusRegister};
use x65alike_emul::{
    image::{load_image, ImageFormat, LoadedImage},
    Machine,
};

const NATIVE_IRQ_VECTOR: usize = 0x00ffee;

static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

pub struct BuildOutput {
    pub image: LoadedImage,
}

pub struct RunResult {
    #[allow(dead_code)]
    pub cpu: Processor,
    pub machine: Machine,
}

pub fn build_mensch_xex(
    name: &str,
    source_path: impl AsRef<Path>,
    cfg_path: impl AsRef<Path>,
) -> BuildOutput {
    let dir = temp_dir(name);
    fs::create_dir_all(&dir).expect("temp dir should be created");

    let asm_path = dir.join(format!("{name}.a65"));
    let obj_path = dir.join(format!("{name}.o"));
    let xex_path = dir.join(format!("{name}.xex"));

    let source =
        fs::read_to_string(source_path.as_ref()).expect("Mensch source should be readable");
    let module = frontend::compile_source(&source).expect("Mensch source should compile to IR");
    let (lowered, _assignment) =
        lower_65816::lower_module_with_storage(&module).expect("Mensch source should lower");
    fs::write(&asm_path, ca65::emit_module(&lowered)).expect("ca65 assembly should be written");

    run_command(
        Command::new("ca65")
            .arg("-g")
            .arg("--cpu")
            .arg("65816")
            .arg(&asm_path)
            .arg("-o")
            .arg(&obj_path),
    );
    run_command(
        Command::new("ld65")
            .arg("-C")
            .arg(cfg_path.as_ref())
            .arg(&obj_path)
            .arg("-o")
            .arg(&xex_path),
    );

    let xex = fs::read(&xex_path).expect("linked XEX should exist");
    let image =
        load_image(&xex_path, &xex, ImageFormat::Auto, 0x020000).expect("linked XEX should load");

    BuildOutput { image }
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
) -> RunResult {
    let mut machine = machine_with_loaded_image(image);
    machine.push_serial_input(serial_input);

    let entry = image.entry.expect("image should include an entry point");
    let mut bus = BusMachine::new(machine);
    let mut cpu = Processor::new();
    cpu.p = StatusRegister::from_byte(0x00, false);
    cpu.pbr = entry.pbr;
    cpu.dbr = 0;
    cpu.pc = entry.pc;
    cpu.s = 0x01fc;

    for _ in 0..max_steps {
        if bus.0.serial_output().len() >= output_len {
            return RunResult {
                cpu,
                machine: bus.into_inner(),
            };
        }
        if !cpu.p.i && bus.0.irq_pending() {
            service_irq(&mut cpu, &mut bus);
        }
        cpu.step(&mut bus);
    }

    panic!(
        "image did not produce {output_len} output bytes; got {:?}; cpu={cpu:?}",
        String::from_utf8_lossy(bus.0.serial_output())
    );
}

pub fn run_command(command: &mut Command) {
    let output = command
        .output()
        .expect("failed to execute assembler/linker command");
    assert!(
        output.status.success(),
        "command failed: {command:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mensch-test-{name}-{}-{nanos}-{id}",
        std::process::id()
    ))
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
    push_u8(cpu, machine, cpu.pbr);
    push_u16(cpu, machine, cpu.pc);
    push_u8(cpu, machine, cpu.p.to_byte(true));
    cpu.pbr = 0;
    cpu.pc = u16::from_le_bytes([
        machine.read(NATIVE_IRQ_VECTOR),
        machine.read(NATIVE_IRQ_VECTOR + 1),
    ]);
    cpu.p.d = false;
    cpu.p.i = true;
}

fn push_u8(cpu: &mut Processor, machine: &mut BusMachine, value: u8) {
    machine.write(cpu.s as usize, value);
    cpu.s = cpu.s.wrapping_sub(1);
}

fn push_u16(cpu: &mut Processor, machine: &mut BusMachine, value: u16) {
    let [low, high] = value.to_le_bytes();
    push_u8(cpu, machine, high);
    push_u8(cpu, machine, low);
}
