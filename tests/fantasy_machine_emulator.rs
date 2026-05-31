use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mensch_systems_tests::{ca65, frontend, lower_65816};
use wdc65816::{HasAddressBus, Processor, StatusRegister};
use x65alike_emul::{
    Machine, RAMBLOCK_REG, ROMBLOCK_REG, USB_UART_CTRL, USB_UART_CTRL_IRQ_RX_NOT_EMPTY,
    USB_UART_DATA, USB_UART_STAT,
};

const ROUTINE_ADDR: usize = 0x020000;
const IRQ_ROUTINE_ADDR: usize = 0x008000;
const RETURN_BANK: u8 = 0x03;
const RETURN_ADDR: u16 = 0x1234;
const NATIVE_IRQ_VECTOR: usize = 0x00ffee;
const DEFAULT_ROMBLOCK_IRQ_VECTOR_PHYSICAL: u32 = 0x083fee;

static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

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

fn run_loaded_routine(machine: Machine) -> (Processor, Machine) {
    run_loaded_routine_at(machine, ROUTINE_ADDR)
}

fn run_loaded_routine_at(machine: Machine, routine_addr: usize) -> (Processor, Machine) {
    let mut machine = BusMachine::new(machine);
    let mut cpu = Processor::new();
    cpu.p = StatusRegister::from_byte(0x00, false);
    cpu.pbr = (routine_addr >> 16) as u8;
    cpu.dbr = 0;
    cpu.pc = routine_addr as u16;
    cpu.s = 0x01fc;

    let return_minus_one = RETURN_ADDR.wrapping_sub(1).to_le_bytes();
    machine.0.write(0x01fd, return_minus_one[0]);
    machine.0.write(0x01fe, return_minus_one[1]);
    machine.0.write(0x01ff, RETURN_BANK);

    for _ in 0..2_000 {
        if cpu.pbr == RETURN_BANK && cpu.pc == RETURN_ADDR {
            return (cpu, machine.into_inner());
        }
        if !cpu.p.i && machine.0.irq_pending() {
            service_irq(&mut cpu, &mut machine);
        }
        cpu.step(&mut machine);
    }

    panic!("fantasy machine routine did not return; cpu={cpu:?}");
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

fn assemble_fixture(name: &str, asm: &str) -> Vec<u8> {
    let dir = temp_dir(name);
    fs::create_dir_all(&dir).expect("temp dir should be created");

    let asm_path = dir.join("fixture.a65");
    let obj_path = dir.join("fixture.o");
    let cfg_path = dir.join("fixture.cfg");
    let bin_path = dir.join("fixture.bin");

    fs::write(&asm_path, asm).expect("fixture assembly should be written");
    fs::write(&cfg_path, linker_config()).expect("fixture linker config should be written");

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
            .arg(&cfg_path)
            .arg(&obj_path)
            .arg("-o")
            .arg(&bin_path),
    );

    fs::read(&bin_path).expect("linked fixture binary should exist")
}

fn assemble_source_fixture(name: &str, source: &str) -> Vec<u8> {
    let module = frontend::compile_source(source).expect("source fixture should compile to IR");
    let (lowered, _assignment) = lower_65816::lower_module_with_storage(&module)
        .expect("source fixture should lower to 65816 assembly");
    assemble_fixture(name, &ca65::emit_module(&lowered))
}

fn assemble_source_with_vectors_fixture(name: &str, source: &str) -> (Vec<u8>, Vec<u8>) {
    let module = frontend::compile_source(source).expect("source fixture should compile to IR");
    let (lowered, _assignment) = lower_65816::lower_module_with_storage(&module)
        .expect("source fixture should lower to 65816 assembly");
    assemble_fixture_outputs(name, &ca65::emit_module(&lowered), irq_linker_config())
}

fn assemble_fixture_outputs(name: &str, asm: &str, cfg: &str) -> (Vec<u8>, Vec<u8>) {
    let dir = temp_dir(name);
    fs::create_dir_all(&dir).expect("temp dir should be created");

    let asm_path = dir.join("fixture.a65");
    let obj_path = dir.join("fixture.o");
    let cfg_path = dir.join("fixture.cfg");
    let code_path = dir.join("code.bin");
    let vectors_path = dir.join("vectors.bin");

    fs::write(&asm_path, asm).expect("fixture assembly should be written");
    let cfg = cfg
        .replace("{CODE_BIN}", &code_path.to_string_lossy())
        .replace("{VECTORS_BIN}", &vectors_path.to_string_lossy());
    fs::write(&cfg_path, cfg).expect("fixture linker config should be written");

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
            .arg(&cfg_path)
            .arg(&obj_path)
            .arg("-o")
            .arg(dir.join("discard.bin")),
    );

    (
        fs::read(&code_path).expect("linked code binary should exist"),
        fs::read(&vectors_path).expect("linked vector binary should exist"),
    )
}

fn run_command(command: &mut Command) {
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

fn linker_config() -> &'static str {
    "MEMORY {\n  CODE: start = $020000, size = $001000, file = %O;\n}\nSEGMENTS {\n  CODE: load = CODE, type = ro;\n}\n"
}

fn irq_linker_config() -> &'static str {
    "MEMORY {\n  CODE: start = $008000, size = $001000, file = \"{CODE_BIN}\";\n  ZEROPAGE: start = $000080, size = $000080, type = rw;\n  BSS: start = $000200, size = $000100, type = rw;\n  VECTORS: start = $00ffee, size = $000002, file = \"{VECTORS_BIN}\";\n}\nSEGMENTS {\n  CODE: load = CODE, type = ro;\n  ZEROPAGE: load = ZEROPAGE, type = zp;\n  BSS: load = BSS, type = bss;\n  VECTORS: load = VECTORS, type = ro;\n}\n"
}

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mensch-fantasy-{name}-{}-{nanos}-{id}",
        std::process::id()
    ))
}

#[test]
fn scripted_serial_program_runs_on_fantasy_machine() {
    let binary = assemble_fixture(
        "serial-echo",
        &format!(
            ".p816\n.code\n.proc main\n\tsep #$20\n\t.a8\n\t.i16\n\tlda #$4f\n\tsta ${serial_data:04x}\n\tlda #$4b\n\tsta ${serial_data:04x}\nwait_rx:\n\tlda ${serial_status:04x}\n\tand #$80\n\tbne wait_rx\n\tlda ${serial_data:04x}\n\tsta ${serial_data:04x}\n\trtl\n.endproc\n",
            serial_data = USB_UART_DATA,
            serial_status = USB_UART_STAT,
        ),
    );

    let mut machine = Machine::new();
    machine.load(ROUTINE_ADDR as u32, &binary);
    machine.push_serial_input(b"!");

    let (_cpu, machine) = run_loaded_routine(machine);

    assert_eq!(machine.serial_output(), b"OK!");
}

#[test]
fn language_serial_program_runs_on_fantasy_machine() {
    let binary = assemble_source_fixture(
        "language-serial-echo",
        &format!(
            "
            module SerialLanguageDemo
            fixed(${serial_status:06x}) volatile var serial_status: u8
            fixed(${serial_data:06x}) volatile var serial_data: u8

            export procedure main()
                serial_data = 'O'
                serial_data = 'K'
                while serial_status & $80 != 0
                end
                serial_data = serial_data
                return
            end
            ",
            serial_data = USB_UART_DATA,
            serial_status = USB_UART_STAT,
        ),
    );

    let mut machine = Machine::new();
    machine.load(ROUTINE_ADDR as u32, &binary);
    machine.push_serial_input(b"!");

    let (_cpu, machine) = run_loaded_routine(machine);

    assert_eq!(machine.serial_output(), b"OK!");
}

#[test]
fn language_irq_vector_handles_usb_uart_rx_interrupt() {
    let (code, vectors) = assemble_source_with_vectors_fixture(
        "language-uart-rx-irq",
        &format!(
            "
            module UartIrqDemo
            vector irq = irq
            fixed(${uart_ctrl:06x}) volatile var uart_ctrl: u8
            fixed(${serial_data:06x}) volatile var serial_data: u8
            fixed($000200) volatile var done: u8

            export procedure main()
                done = 0
                uart_ctrl = ${rx_irq:02x}
                while done == 0
                end
                return
            end

            interrupt procedure irq()
                serial_data = serial_data
                done = 1
                return
            end
            ",
            uart_ctrl = USB_UART_CTRL,
            serial_data = USB_UART_DATA,
            rx_irq = USB_UART_CTRL_IRQ_RX_NOT_EMPTY,
        ),
    );

    let mut machine = Machine::new();
    machine.load(IRQ_ROUTINE_ADDR as u32, &code);
    machine.load(DEFAULT_ROMBLOCK_IRQ_VECTOR_PHYSICAL, &vectors);
    machine.push_serial_input(b"!");

    let (_cpu, machine) = run_loaded_routine_at(machine, IRQ_ROUTINE_ADDR);

    assert_eq!(machine.serial_output(), b"!");
    assert_eq!(machine.byte(0x000200), 1);
}

#[test]
fn bank_registers_and_frames_match_x65_shape() {
    let mut machine = Machine::new();

    machine.write(RAMBLOCK_REG, 0);
    machine.write(0x00a123, 0x11);
    assert_eq!(machine.byte(0x100123), 0x11);

    machine.write(ROMBLOCK_REG, 2);
    machine.write(0x00c456, 0x22);
    assert_eq!(machine.byte(0x088456), 0x22);
}
