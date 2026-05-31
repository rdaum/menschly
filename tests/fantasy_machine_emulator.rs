use std::fs;
use std::process::Command;

use mensch_systems_tests::{ca65, frontend, lower_65816, x65_runner};
use wdc65816::{HasAddressBus, Processor, StatusRegister};
use x65alike_emul::{
    Machine, RAMBLOCK_REG, ROMBLOCK_REG, USB_UART_CTRL, USB_UART_CTRL_IRQ_RX_NOT_EMPTY,
    USB_UART_CTRL_IRQ_TX_NOT_FULL, USB_UART_DATA, USB_UART_STAT,
};

use x65_runner::{
    build_mensch_xex_sources, machine_with_loaded_image, run_image_until_output, temp_dir,
};

const ROUTINE_ADDR: usize = 0x020000;
const IRQ_ROUTINE_ADDR: usize = 0x008000;
const RETURN_BANK: u8 = 0x03;
const RETURN_ADDR: u16 = 0x1234;
const NATIVE_IRQ_VECTOR: usize = 0x00ffee;
const DEFAULT_ROMBLOCK_IRQ_VECTOR_PHYSICAL: u32 = 0x083fee;
const HELLORD_OVERFLOW_ADDR: u32 = 0x000280;
const HELLORD_TX_OVERFLOW_ADDR: u32 = 0x000282;
const TX_BUFFERED_MARKER_ADDR: u32 = 0x000290;

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
    x65_runner::run_command(command).expect("assembler/linker command should succeed");
}

fn linker_config() -> &'static str {
    "MEMORY {\n  CODE: start = $020000, size = $001000, file = %O;\n}\nSEGMENTS {\n  CODE: load = CODE, type = ro;\n}\n"
}

fn irq_linker_config() -> &'static str {
    "MEMORY {\n  CODE: start = $008000, size = $001000, file = \"{CODE_BIN}\";\n  ZEROPAGE: start = $000080, size = $000080, type = rw;\n  BSS: start = $000200, size = $000100, type = rw;\n  VECTORS: start = $00ffee, size = $000002, file = \"{VECTORS_BIN}\";\n}\nSEGMENTS {\n  CODE: load = CODE, type = ro;\n  ZEROPAGE: load = ZEROPAGE, type = zp;\n  BSS: load = BSS, type = bss;\n  VECTORS: load = VECTORS, type = ro;\n}\n"
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

            export far procedure main()
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
            var rx_head: u16
            var rx_tail: u16
            var rx_buffer: [4]u8

            export far procedure main()
                rx_head = 0
                rx_tail = 0
                uart_ctrl = ${rx_irq:02x}
                while rx_head == rx_tail
                end
                serial_data = rx_buffer[rx_tail]
                rx_tail = rx_tail + 1 & 3
                return
            end

            interrupt procedure irq()
                rx_buffer[rx_head] = serial_data
                rx_head = rx_head + 1 & 3
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
    assert_eq!(machine.byte(0x000202), 1);
    assert_eq!(machine.byte(0x000204), b'!');
    assert!(!machine.irq_pending());
}

#[test]
fn language_irq_preserves_interrupted_accumulator_by_default() {
    let (code, vectors) = assemble_source_with_vectors_fixture(
        "language-uart-irq-preserves-a",
        &format!(
            "
            module UartIrqPreserveDemo
            vector irq = irq
            fixed(${uart_ctrl:06x}) volatile var uart_ctrl: u8
            fixed(${serial_data:06x}) volatile var serial_data: u8
            fixed($000292) volatile var seen: u8

            export far procedure main()
                asm \"sei\"
                uart_ctrl = ${rx_irq:02x}
                asm \"lda #$5a\"
                asm \"cli\"
                asm \"nop\"
                asm \"sta ${serial_data:06x}\"
                return
            end

            interrupt procedure irq()
                seen = serial_data
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

    assert_eq!(machine.byte(0x000292), b'!');
    assert_eq!(machine.serial_output(), b"Z");
}

#[test]
fn hellord_example_boots_as_interrupt_driven_xex() {
    let build = build_hellord_example();
    let expected = b"Hellord!\r\nabc123\n";

    let run = run_image_until_output(&build.image, b"abc123\n", expected.len(), 20_000)
        .expect("Hellord image should produce expected output");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_example_flags_full_rx_ring_and_drops_input() {
    let build = build_hellord_example();
    let input = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let output_len = b"Hellord!\r\n".len() + 8;

    let run = run_image_until_output(&build.image, input, output_len, 50_000)
        .expect("Hellord image should produce overflow test output");

    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 1);
}

#[test]
fn serial_write_buffers_until_tx_irq_drains_output() {
    let dir = temp_dir("buffered-tx-irq");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let main_path = dir.join("buffered-tx.mensch");
    let cfg_path = dir.join("image.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    fs::write(
        &main_path,
        "
        module BufferedTxDemo
        application main
        vector irq = serial_irq
        fixed($000290) volatile var tx_marker: u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external procedure serial_write(value: u8)

        export procedure main()
            serial_init()
            asm \"sei\"
            serial_write('B')
            serial_write('U')
            serial_write('F')
            serial_write('F')
            serial_write('E')
            serial_write('R')
            tx_marker = 1
            while tx_marker == 1
            end
            asm \"cli\"
            while 1 != 0
            end
        end
        ",
    )
    .expect("buffered TX demo should be written");
    fs::write(&cfg_path, xex_linker_config()).expect("linker config should be written");

    let build = build_mensch_xex_sources(
        "buffered-tx-irq",
        &[main_path, repo_root.join("examples/serial-ring.mensch")],
        &cfg_path,
    )
    .expect("buffered TX demo should build");

    let mut bus = BusMachine::new(machine_with_loaded_image(&build.image));
    let mut cpu = Processor::new();
    let entry = build.image.entry.expect("XEX should include a run address");
    cpu.p = StatusRegister::from_byte(0x00, false);
    cpu.pbr = entry.pbr;
    cpu.dbr = 0;
    cpu.pc = entry.pc;
    cpu.s = 0x01fc;

    for _ in 0..20_000 {
        if bus.0.byte(TX_BUFFERED_MARKER_ADDR) == 1 {
            break;
        }
        if !cpu.p.i && bus.0.irq_pending() {
            service_irq(&mut cpu, &mut bus);
        }
        cpu.step(&mut bus);
    }

    assert_eq!(bus.0.byte(TX_BUFFERED_MARKER_ADDR), 1);
    assert_eq!(bus.0.serial_output(), b"");
    assert_eq!(
        bus.0.read(USB_UART_CTRL),
        USB_UART_CTRL_IRQ_RX_NOT_EMPTY | USB_UART_CTRL_IRQ_TX_NOT_FULL
    );
    assert!(bus.0.irq_pending());

    bus.0.write(TX_BUFFERED_MARKER_ADDR, 0);
    for _ in 0..50_000 {
        if !cpu.p.i && bus.0.irq_pending() {
            service_irq(&mut cpu, &mut bus);
        }
        cpu.step(&mut bus);
        if bus.0.serial_output() == b"BUFFER"
            && bus.0.read(USB_UART_CTRL) == USB_UART_CTRL_IRQ_RX_NOT_EMPTY
        {
            break;
        }
    }

    assert_eq!(bus.0.serial_output(), b"BUFFER");
    assert_eq!(bus.0.read(USB_UART_CTRL), USB_UART_CTRL_IRQ_RX_NOT_EMPTY);
    assert!(!bus.0.irq_pending());
}

fn build_hellord_example() -> x65_runner::BuildOutput {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    build_mensch_xex_sources(
        "hellord-os",
        &[
            repo_root.join("examples/hellord-os.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
        ],
        repo_root.join("examples/hellord-os-x65.cfg"),
    )
    .expect("Hellord example should build")
}

#[test]
fn non_exported_routine_is_not_callable_across_modules() {
    let dir = temp_dir("private-cross-module-link");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let callee_path = dir.join("callee.mensch");
    let cfg_path = dir.join("image.cfg");

    fs::write(
        &caller_path,
        "
        module Caller
        application main
        external procedure private_helper()

        procedure main()
            private_helper()
            return
        end
        ",
    )
    .expect("caller source should be written");
    fs::write(
        &callee_path,
        "
        module Callee
        procedure private_helper()
            return
        end
        ",
    )
    .expect("callee source should be written");
    fs::write(&cfg_path, xex_linker_config()).expect("linker config should be written");

    let error = match build_mensch_xex_sources(
        "private-cross-module-link",
        &[caller_path, callee_path],
        &cfg_path,
    ) {
        Ok(_) => panic!("linking a non-exported cross-module routine should fail"),
        Err(error) => error,
    };

    assert!(
        error.contains("private_helper"),
        "linker error should name unresolved private helper: {error}"
    );
}

#[test]
fn external_multi_argument_call_links_and_runs() {
    let dir = temp_dir("external-multi-argument-link");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let callee_path = dir.join("callee.mensch");
    let cfg_path = dir.join("image.cfg");

    fs::write(
        &caller_path,
        "
        module Caller
        application main
        external procedure store_pair(first: u8, second: u16)

        procedure main()
            store_pair('X', $3456)
            return
        end
        ",
    )
    .expect("caller source should be written");
    fs::write(
        &callee_path,
        "
        module Callee
        fixed($009f57) volatile var serial_data: u8
        fixed($000284) var word_out: u16

        export procedure store_pair(first: u8, second: u16)
            word_out = second
            serial_data = first
            return
        end
        ",
    )
    .expect("callee source should be written");
    fs::write(&cfg_path, xex_linker_config()).expect("linker config should be written");

    let build = build_mensch_xex_sources(
        "external-multi-argument-link",
        &[caller_path, callee_path],
        &cfg_path,
    )
    .expect("external multi-argument image should build");
    let run = run_image_until_output(&build.image, b"", 1, 20_000)
        .expect("external multi-argument image should output a byte");

    assert_eq!(run.output, b"X");
    assert_eq!(run.machine.byte(0x000284), 0x56);
    assert_eq!(run.machine.byte(0x000285), 0x34);
}

#[test]
fn external_reentrant_stack_argument_call_links_and_runs() {
    let dir = temp_dir("external-reentrant-stack-argument-link");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let callee_path = dir.join("callee.mensch");
    let cfg_path = dir.join("image.cfg");

    fs::write(
        &caller_path,
        "
        module Caller
        application main
        external reentrant procedure store_pair(first: u8, second: u16)

        procedure main()
            store_pair('R', $2345)
            return
        end
        ",
    )
    .expect("caller source should be written");
    fs::write(
        &callee_path,
        "
        module Callee
        fixed($009f57) volatile var serial_data: u8
        fixed($000286) var first_out: u8
        fixed($000288) var second_out: u16
        fixed($00028a) var local_out: u8

        export reentrant procedure store_pair(first: u8, second: u16)
            var derived: u8
            derived = first + 1
            first_out = first
            second_out = second
            local_out = derived
            serial_data = derived
            return
        end
        ",
    )
    .expect("callee source should be written");
    fs::write(&cfg_path, xex_linker_config()).expect("linker config should be written");

    let build = build_mensch_xex_sources(
        "external-reentrant-stack-argument-link",
        &[caller_path, callee_path],
        &cfg_path,
    )
    .expect("external reentrant stack-argument image should build");
    let run = run_image_until_output(&build.image, b"", 1, 20_000)
        .expect("external reentrant stack-argument image should output a byte");

    assert_eq!(run.output, b"S");
    assert_eq!(run.machine.byte(0x000286), b'R');
    assert_eq!(run.machine.byte(0x000288), 0x45);
    assert_eq!(run.machine.byte(0x000289), 0x23);
    assert_eq!(run.machine.byte(0x00028a), b'S');
}

fn xex_linker_config() -> &'static str {
    "MEMORY {\n  ZP: start = $000080, size = $000080, type = rw;\n  CODE: start = $008000, size = $001000, file = %O;\n  META: start = $008f00, size = $000100, file = %O;\n  BSS: start = $009000, size = $000f00, type = rw;\n  VECTORS: start = $00ffee, size = $000002, file = %O;\n}\nFILES {\n  %O: format = atari;\n}\nFORMATS {\n  atari: runad = main;\n}\nSEGMENTS {\n  ZEROPAGE: load = ZP, type = zp;\n  CODE: load = CODE, type = ro;\n  MENSCH_META: load = META, type = ro, optional = yes;\n  BSS: load = BSS, type = bss;\n  VECTORS: load = VECTORS, type = ro, optional = yes;\n}\n"
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
