use std::fs;
use std::process::Command;

use mensch_systems_tests::{ca65, frontend, lower_65816, storage, x65_runner};
use wdc65816::{HasAddressBus, Processor, StatusRegister};
use x65alike_emul::{
    image::CpuStart, Machine, RAMBLOCK_REG, ROMBLOCK_REG, USB_UART_CTRL,
    USB_UART_CTRL_IRQ_RX_NOT_EMPTY, USB_UART_CTRL_IRQ_TX_NOT_FULL, USB_UART_DATA, USB_UART_STAT,
};

use x65_runner::{
    build_mensch_x65_image_sources, build_mensch_xex_sources, machine_with_loaded_image,
    run_image_until_output, temp_dir, BlockDeviceConfig, RunConfig, X65ImageBlock,
};

const ROUTINE_ADDR: usize = 0x020000;
const IRQ_ROUTINE_ADDR: usize = 0x008000;
const RETURN_BANK: u8 = 0x03;
const RETURN_ADDR: u16 = 0x1234;
const NATIVE_IRQ_VECTOR: usize = 0x00ffee;
const DEFAULT_ROMBLOCK_NATIVE_VECTOR_PHYSICAL: u32 = 0x083fe4;
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
    "MEMORY {\n  CODE: start = $008000, size = $001000, file = \"{CODE_BIN}\";\n  ZEROPAGE: start = $000080, size = $000080, type = rw;\n  BSS: start = $000200, size = $000100, type = rw;\n  VECTORS: start = $00ffe4, size = $00000c, file = \"{VECTORS_BIN}\";\n}\nSEGMENTS {\n  CODE: load = CODE, type = ro;\n  ZEROPAGE: load = ZEROPAGE, type = zp;\n  BSS: load = BSS, type = bss;\n  VECTOR_COP: load = VECTORS, type = ro, optional = yes, offset = $0000;\n  VECTOR_BRK: load = VECTORS, type = ro, optional = yes, offset = $0002;\n  VECTOR_ABORT: load = VECTORS, type = ro, optional = yes, offset = $0004;\n  VECTOR_NMI: load = VECTORS, type = ro, optional = yes, offset = $0006;\n  VECTOR_IRQ: load = VECTORS, type = ro, optional = yes, offset = $000a;\n  VECTORS: load = VECTORS, type = ro, optional = yes;\n}\n"
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
fn language_banked_x65_image_loads_and_runs_from_24_bit_entry() {
    let dir = temp_dir("language-banked-x65-image");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let source_path = dir.join("banked.mensch");
    let cfg_path = dir.join("banked.cfg");

    fs::write(
        &source_path,
        format!(
            "
            module BankedImage
            application main
            metadata code_segment = \"BANK1\"
            fixed(${serial_data:06x}) volatile var serial_data: u8

            procedure main()
                serial_data = 'B'
                serial_data = '1'
                asm \"stp\"
                return
            end
            ",
            serial_data = USB_UART_DATA,
        ),
    )
    .expect("banked source should be written");
    fs::write(
        &cfg_path,
        "MEMORY {\n  ZP: start = $000080, size = $000080, type = rw;\n  BANK1: start = $01c000, size = $004000, file = \"bank1.bin\";\n  META: start = $01ff00, size = $000100, file = \"meta.bin\";\n  BSS: start = $018000, size = $001000, type = rw;\n}\nSEGMENTS {\n  ZEROPAGE: load = ZP, type = zp;\n  BANK1: load = BANK1, type = ro;\n  MENSCH_META: load = META, type = ro, optional = yes;\n  BSS: load = BSS, type = bss;\n}\n",
    )
    .expect("banked linker config should be written");

    let build = build_mensch_x65_image_sources(
        "language-banked-x65-image",
        &[source_path],
        &cfg_path,
        CpuStart {
            pbr: 0x01,
            pc: 0xc000,
        },
        &[X65ImageBlock {
            address: 0x01c000,
            path: "bank1.bin".into(),
        }],
    )
    .expect("banked x65 image should build");

    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 50_000,
            stop_after_output_len: Some(2),
            live_input: None,
            block_device: None,
        },
        &mut Vec::new(),
    )
    .expect("banked x65 image should run");

    assert_eq!(run.output, b"B1");
}

#[test]
fn language_x65_image_calls_far_procedure_in_another_bank() {
    let dir = temp_dir("language-x65-far-bank-call");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let callee_path = dir.join("callee.mensch");
    let cfg_path = dir.join("banked-call.cfg");

    fs::write(
        &caller_path,
        format!(
            "
            module BankedCaller
            application main
            fixed(${serial_data:06x}) volatile var serial_data: u8

            external far procedure banked_write(value: u8)

            export far procedure main()
                serial_data = 'A'
                banked_write('B')
                serial_data = 'C'
                asm \"stp\"
                return
            end
            ",
            serial_data = USB_UART_DATA,
        ),
    )
    .expect("caller source should be written");
    fs::write(
        &callee_path,
        format!(
            "
            module BankedCallee
            metadata code_segment = \"BANK1\"
            fixed(${serial_data:06x}) volatile var serial_data: u8

            export far procedure banked_write(value: u8)
                serial_data = value
                return
            end
            ",
            serial_data = USB_UART_DATA,
        ),
    )
    .expect("callee source should be written");
    fs::write(
        &cfg_path,
        "MEMORY {\n  ZP: start = $000080, size = $000080, type = rw;\n  CODE: start = $028000, size = $002000, file = \"code.bin\";\n  BANK1: start = $01c000, size = $004000, file = \"bank1.bin\";\n  META: start = $02f000, size = $000100, file = \"meta.bin\";\n  BSS: start = $018000, size = $001000, type = rw;\n}\nSEGMENTS {\n  ZEROPAGE: load = ZP, type = zp;\n  CODE: load = CODE, type = ro;\n  BANK1: load = BANK1, type = ro;\n  MENSCH_META: load = META, type = ro, optional = yes;\n  BSS: load = BSS, type = bss;\n}\n",
    )
    .expect("banked call linker config should be written");

    let build = build_mensch_x65_image_sources(
        "language-x65-far-bank-call",
        &[caller_path, callee_path],
        &cfg_path,
        CpuStart {
            pbr: 0x02,
            pc: 0x8000,
        },
        &[
            X65ImageBlock {
                address: 0x028000,
                path: "code.bin".into(),
            },
            X65ImageBlock {
                address: 0x01c000,
                path: "bank1.bin".into(),
            },
        ],
    )
    .expect("banked far-call x65 image should build");

    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 50_000,
            stop_after_output_len: Some(3),
            live_input: None,
            block_device: None,
        },
        &mut Vec::new(),
    )
    .expect("banked far-call x65 image should run");

    assert_eq!(run.output, b"ABC");
}

#[test]
fn language_bios_bconout_runs_through_cop_syscall() {
    let dir = temp_dir("language-bios-bconout-cop");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let traps_path = dir.join("bios_traps.mensch");
    let cfg_path = dir.join("image.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    fs::write(
        &caller_path,
        "
        module BiosSyscallCaller
        application main
        vector irq = serial_irq
        fixed($000294) volatile var status: u16
        fixed($0002a6) volatile var input_status: u16
        fixed($0002a8) volatile var input_value: u8
        fixed($000296) volatile var first_device: u16
        fixed($000298) volatile var next_device: u16
        fixed($00029a) volatile var parent_device: u16
        fixed($00029c) volatile var child_device: u16
        fixed($00029e) volatile var device_kind: u16
        fixed($0002a0) volatile var device_flags: u16
        fixed($0002a2) volatile var device_driver: u16
        fixed($0002a4) volatile var bad_device_kind: u16
        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant function bios_bconstat(dev: u16): u16
        external reentrant function bios_bconin(dev: u16): u8
        external procedure bios_bconout(dev: u16, ch: u16)
        external reentrant function bios_bcostat(dev: u16): u16
        external reentrant function bios_device_first(): u16
        external reentrant function bios_device_next(dev: u16): u16
        external reentrant function bios_device_parent(dev: u16): u16
        external reentrant function bios_device_child(dev: u16): u16
        external reentrant function bios_device_kind(dev: u16): u16
        external reentrant function bios_device_flags(dev: u16): u16
        external reentrant function bios_device_driver(dev: u16): u16

        procedure main()
            serial_init()
            bios_bconout(2, $0042)
            bios_bconout(1, $0058)
            status = bios_bcostat(2)
            input_status = bios_bconstat(2)
            input_value = bios_bconin(2)
            first_device = bios_device_first()
            next_device = bios_device_next(first_device)
            parent_device = bios_device_parent(first_device)
            child_device = bios_device_child(first_device)
            device_kind = bios_device_kind(first_device)
            device_flags = bios_device_flags(first_device)
            device_driver = bios_device_driver(first_device)
            bad_device_kind = bios_device_kind(1)
            bios_bconout(2, $0021)
            return
        end
        ",
    )
    .expect("caller source should be written");
    fs::write(&traps_path, bios_only_traps_source()).expect("BIOS trap source should be written");
    fs::write(&cfg_path, cop_xex_linker_config()).expect("linker config should be written");

    let build = build_mensch_xex_sources(
        "language-bios-bconout-cop",
        &[
            caller_path,
            repo_root.join("examples/hellord-bios.mensch"),
            traps_path,
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("BIOS syscall image should build");

    let run = run_image_until_output(&build.image, b"?", 2, 50_000)
        .expect("BIOS syscall image should write two console bytes");

    assert_eq!(run.output, b"B!");
    assert_eq!(run.machine.byte(0x000294), 0xff);
    assert_eq!(run.machine.byte(0x000295), 0xff);
    assert_eq!(run.machine.byte(0x0002a6), 0xff);
    assert_eq!(run.machine.byte(0x0002a7), 0xff);
    assert_eq!(run.machine.byte(0x0002a8), b'?');
    assert_eq!(run.machine.byte(0x000296), 0x02);
    assert_eq!(run.machine.byte(0x000297), 0x00);
    assert_eq!(run.machine.byte(0x000298), 0x08);
    assert_eq!(run.machine.byte(0x000299), 0x00);
    assert_eq!(run.machine.byte(0x00029a), 0x00);
    assert_eq!(run.machine.byte(0x00029b), 0x00);
    assert_eq!(run.machine.byte(0x00029c), 0x00);
    assert_eq!(run.machine.byte(0x00029d), 0x00);
    assert_eq!(run.machine.byte(0x00029e), 0x01);
    assert_eq!(run.machine.byte(0x00029f), 0x00);
    assert_eq!(run.machine.byte(0x0002a0), 0x03);
    assert_eq!(run.machine.byte(0x0002a1), 0x00);
    assert_eq!(run.machine.byte(0x0002a2), 0x01);
    assert_eq!(run.machine.byte(0x0002a3), 0x00);
    assert_eq!(run.machine.byte(0x0002a4), 0x00);
    assert_eq!(run.machine.byte(0x0002a5), 0x00);
}

#[test]
fn language_bios_block_write_updates_attached_disk_image() {
    let dir = temp_dir("language-bios-block-write");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let traps_path = dir.join("bios_traps.mensch");
    let cfg_path = dir.join("image.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    fs::write(
        &caller_path,
        "
        module BiosBlockWriteCaller
        application main
        vector irq = serial_irq
        const BIOS_BLOCK0 = $0008
        const WRITE_BUFFER = $9400
        fixed($0002b2) volatile var write_status: u16
        fixed($009400) var write_buffer: [512]u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external procedure bios_bconout(dev: u16, ch: u16)
        external reentrant procedure bios_block_set_lba(dev: u16, lba: u16)
        external reentrant function bios_block_write(dev: u16, buffer: near ^u8): u16

        procedure main()
            serial_init()
            write_buffer[0] = 'W'
            write_buffer[1] = 'R'
            write_buffer[2] = 'I'
            write_buffer[3] = 'T'
            write_buffer[4] = 'E'
            bios_block_set_lba(BIOS_BLOCK0, 1)
            write_status = bios_block_write(BIOS_BLOCK0, WRITE_BUFFER)
            bios_bconout(2, '!')
            return
        end
        ",
    )
    .expect("caller source should be written");
    fs::write(&traps_path, bios_only_traps_source()).expect("BIOS trap source should be written");
    fs::write(&cfg_path, cop_xex_linker_config()).expect("linker config should be written");

    let build = build_mensch_xex_sources(
        "language-bios-block-write",
        &[
            caller_path,
            repo_root.join("examples/hellord-bios.mensch"),
            traps_path,
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("BIOS block write image should build");

    let mut output = Vec::new();
    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 500_000,
            stop_after_output_len: Some(1),
            live_input: None,
            block_device: Some(BlockDeviceConfig {
                image: vec![0; 1024],
                read_only: false,
            }),
        },
        &mut output,
    )
    .expect("BIOS block write image should run");

    let disk = run
        .machine
        .block_device_image()
        .expect("block image should remain attached");
    assert_eq!(run.output, b"!");
    assert_eq!(&disk[512..517], b"WRITE");
    assert_eq!(run.machine.byte(0x0002b2), 0);
    assert_eq!(run.machine.byte(0x0002b3), 0);
}

#[test]
fn language_fat_sector_helpers_use_dos_drive_mapping() {
    let dir = temp_dir("language-fat-sector-helpers");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let cfg_path = dir.join("image.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::write(&cfg_path, large_integration_xex_linker_config())
        .expect("large integration linker config should be written");

    fs::write(
        &caller_path,
        "
        module FatSectorCaller
        application main
        vector irq = serial_irq
        const READ_BUFFER = $9800
        const WRITE_BUFFER = $9a00
        fixed($0002e4) volatile var mount_a_status: u16
        fixed($0002e8) volatile var write_status: u16
        fixed($009800) var read_buffer: [512]u8
        fixed($009a00) var write_buffer: [512]u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant procedure gemdos_init()
        external reentrant procedure gemdos_cconout(ch: u8)
        external reentrant function dos_mount_fat_drive(drive: u16): u16
        external reentrant function fat_read_sector(drive: u16, lba: u16, buffer: near ^u8): u16
        external reentrant function fat_write_sector(drive: u16, lba: u16, buffer: near ^u8): u16

        procedure main()
            serial_init()
            gemdos_init()
            mount_a_status = dos_mount_fat_drive(0)
            write_buffer[0] = 'F'
            write_buffer[1] = 'A'
            write_buffer[2] = 'T'
            write_buffer[3] = '!'
            write_status = fat_write_sector(0, 1, WRITE_BUFFER)
            gemdos_cconout('!')
            return
        end
        ",
    )
    .expect("caller source should be written");

    let build = build_mensch_xex_sources(
        "language-fat-sector-helpers",
        &[
            caller_path,
            repo_root.join("examples/hellord-dos.mensch"),
            repo_root.join("examples/hellord-bios.mensch"),
            repo_root.join("examples/hellord-gemdos.mensch"),
            repo_root.join("examples/hellord-traps.mensch"),
            repo_root.join("examples/hellord-console.mensch"),
            repo_root.join("examples/hellord-files.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("FAT helper image should build");

    let mut disk = vec![0; 1024];
    disk[0] = b'M';
    disk[1] = b'Z';
    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 600_000,
            stop_after_output_len: Some(1),
            live_input: None,
            block_device: Some(BlockDeviceConfig {
                image: disk,
                read_only: false,
            }),
        },
        &mut Vec::new(),
    )
    .expect("FAT helper image should run");

    let disk = run
        .machine
        .block_device_image()
        .expect("block image should remain attached");
    assert_eq!(run.output, b"!");
    assert_eq!(run.machine.byte(0x0002e4), 0);
    assert_eq!(run.machine.byte(0x0002e5), 0);
    assert_eq!(run.machine.byte(0x0002e8), 0);
    assert_eq!(run.machine.byte(0x0002e9), 0);
    assert_eq!(&disk[512..516], b"FAT!");
}

#[test]
fn language_gemdos_fsfirst_enumerates_fat_root_directory() {
    let dir = temp_dir("language-gemdos-fat-root-dir");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let cfg_path = dir.join("image.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::write(&cfg_path, large_integration_xex_linker_config())
        .expect("large integration linker config should be written");

    fs::write(
        &caller_path,
        "
        module GemdosFatDirCaller
        application main
        vector irq = serial_irq
        fixed($0002f0) volatile var mount_status: u16
        fixed($0002f2) volatile var first_status: u16
        fixed($0002f4) volatile var second_status: u16
        fixed($0002f6) volatile var done_status: u16
        const FIRST_DTA = $9800
        const SECOND_DTA = $9840
        fixed($009800) var first_dta: [32]u8
        fixed($009840) var second_dta: [32]u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant procedure gemdos_init()
        external reentrant function dos_mount_fat_drive(drive: u16): u16
        external reentrant function gemdos_fsfirst(pattern: near ^u8, dta: near ^u8): u16
        external reentrant function gemdos_fsnext(dta: near ^u8): u16
        external reentrant procedure gemdos_cconout(ch: u8)

        procedure main()
            serial_init()
            gemdos_init()
            mount_status = dos_mount_fat_drive(0)
            first_status = gemdos_fsfirst(\"*.*\", FIRST_DTA)
            second_status = gemdos_fsnext(SECOND_DTA)
            done_status = gemdos_fsnext(SECOND_DTA)
            gemdos_cconout('!')
            return
        end
        ",
    )
    .expect("caller source should be written");

    let build = build_mensch_xex_sources(
        "language-gemdos-fat-root-dir",
        &[
            caller_path,
            repo_root.join("examples/hellord-dos.mensch"),
            repo_root.join("examples/hellord-bios.mensch"),
            repo_root.join("examples/hellord-gemdos.mensch"),
            repo_root.join("examples/hellord-traps.mensch"),
            repo_root.join("examples/hellord-console.mensch"),
            repo_root.join("examples/hellord-files.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("GEMDOS FAT directory image should build");

    let mut disk = vec![0; 2048];
    disk[11] = 0x00;
    disk[12] = 0x02;
    disk[13] = 1;
    disk[14] = 1;
    disk[16] = 1;
    disk[17] = 16;
    disk[22] = 1;
    let root = 1024;
    disk[root..root + 11].copy_from_slice(b"FILE    TXT");
    disk[root + 11] = 0x20;
    disk[root + 28] = 5;
    disk[root + 32..root + 43].copy_from_slice(b"SUBDIR     ");
    disk[root + 32 + 11] = 0x10;

    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 800_000,
            stop_after_output_len: Some(1),
            live_input: None,
            block_device: Some(BlockDeviceConfig {
                image: disk,
                read_only: false,
            }),
        },
        &mut Vec::new(),
    )
    .expect("GEMDOS FAT directory image should run");

    assert_eq!(run.output, b"!");
    assert_eq!(run.machine.byte(0x0002f0), 0);
    assert_eq!(run.machine.byte(0x0002f1), 0);
    assert_eq!(run.machine.byte(0x0002f2), 0);
    assert_eq!(run.machine.byte(0x0002f3), 0);
    assert_eq!(run.machine.byte(0x0002f4), 0);
    assert_eq!(run.machine.byte(0x0002f5), 0);
    assert_eq!(run.machine.byte(0x0002f6), 0xfe);
    assert_eq!(run.machine.byte(0x0002f7), 0xff);
    assert_eq!(run.machine.byte(0x009800), 0);
    assert_eq!(run.machine.byte(0x009801), 5);
    assert_eq!(run.machine.byte(0x009802), 0);
    assert_eq!(
        &[
            run.machine.byte(0x009803),
            run.machine.byte(0x009804),
            run.machine.byte(0x009805),
            run.machine.byte(0x009806),
            run.machine.byte(0x009807),
            run.machine.byte(0x009808),
            run.machine.byte(0x009809),
            run.machine.byte(0x00980a),
            run.machine.byte(0x00980b),
        ],
        b"FILE.TXT\0"
    );
    assert_eq!(run.machine.byte(0x009840), 0x10);
    assert_eq!(run.machine.byte(0x009841), 0);
    assert_eq!(run.machine.byte(0x009842), 0);
    assert_eq!(
        &[
            run.machine.byte(0x009843),
            run.machine.byte(0x009844),
            run.machine.byte(0x009845),
            run.machine.byte(0x009846),
            run.machine.byte(0x009847),
            run.machine.byte(0x009848),
            run.machine.byte(0x009849),
        ],
        b"SUBDIR\0"
    );
}

#[test]
fn language_gemdos_fread_reads_fat_root_file() {
    let dir = temp_dir("language-gemdos-fat-file-read");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let cfg_path = dir.join("fat-file-read.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::write(&cfg_path, large_integration_xex_linker_config())
        .expect("large integration linker config should be written");

    fs::write(
        &caller_path,
        "
        module GemdosFatFileCaller
        application main
        vector irq = serial_irq
        const READ_BUFFER = $9400
        fixed($009400) var read_buffer: [1028]u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant procedure gemdos_init()
        external reentrant function dos_mount_fat_drive(drive: u16): u16
        external reentrant function gemdos_fopen(path: near ^u8, mode: u16): u16
        external reentrant function gemdos_fread(handle: u16, count: u16, buffer: near ^u8): u16
        external reentrant procedure gemdos_cconout(ch: u8)

        procedure main()
            var handle: u16
            var status: u16
            serial_init()
            gemdos_init()
            status = dos_mount_fat_drive(0)
            handle = gemdos_fopen(\"FILE.TXT\", 0)
            status = gemdos_fread(handle, 1026, READ_BUFFER)
            gemdos_cconout('!')
            return
        end
        ",
    )
    .expect("caller source should be written");

    let build = build_mensch_xex_sources(
        "language-gemdos-fat-file-read",
        &[
            caller_path,
            repo_root.join("examples/hellord-dos.mensch"),
            repo_root.join("examples/hellord-bios.mensch"),
            repo_root.join("examples/hellord-gemdos.mensch"),
            repo_root.join("examples/hellord-traps.mensch"),
            repo_root.join("examples/hellord-console.mensch"),
            repo_root.join("examples/hellord-files.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("GEMDOS FAT file image should build");

    let mut disk = vec![0; 4096];
    disk[11] = 0x00;
    disk[12] = 0x02;
    disk[13] = 1;
    disk[14] = 1;
    disk[16] = 1;
    disk[17] = 16;
    disk[22] = 1;
    let root = 1024;
    disk[516] = 4;
    disk[520] = 6;
    disk[root..root + 11].copy_from_slice(b"FILE    TXT");
    disk[root + 11] = 0x20;
    disk[root + 26] = 2;
    disk[root + 28] = 0x02;
    disk[root + 29] = 0x04;
    disk[1536..1542].copy_from_slice(b"HELLO!");
    disk[2560] = b'O';
    disk[2561] = b'K';
    disk[3584] = b'3';
    disk[3585] = b'!';

    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 30_000_000,
            stop_after_output_len: Some(1),
            live_input: None,
            block_device: Some(BlockDeviceConfig {
                image: disk,
                read_only: false,
            }),
        },
        &mut Vec::new(),
    )
    .expect("GEMDOS FAT file image should run");

    assert_eq!(run.output, b"!");
    assert_eq!(run.machine.byte(0x009600), b'O');
    assert_eq!(run.machine.byte(0x009601), b'K');
    assert_eq!(run.machine.byte(0x009800), b'3');
    assert_eq!(run.machine.byte(0x009801), b'!');
    assert_eq!(
        &[
            run.machine.byte(0x009400),
            run.machine.byte(0x009401),
            run.machine.byte(0x009402),
            run.machine.byte(0x009403),
            run.machine.byte(0x009404),
            run.machine.byte(0x009405),
        ],
        b"HELLO!"
    );
}

#[test]
fn language_gemdos_fwrite_updates_fat_root_file_sector() {
    let dir = temp_dir("language-gemdos-fat-file-write");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let cfg_path = dir.join("fat-file-write.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::write(&cfg_path, large_integration_xex_linker_config())
        .expect("large integration linker config should be written");

    fs::write(
        &caller_path,
        "
        module GemdosFatFileWriteCaller
        application main
        vector irq = serial_irq
        const WRITE_BUFFER = $9400
        fixed($000304) volatile var written: u16
        fixed($009400) var write_buffer: [512]u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant procedure gemdos_init()
        external reentrant function dos_mount_fat_drive(drive: u16): u16
        external reentrant function gemdos_fopen(path: near ^u8, mode: u16): u16
        external reentrant function gemdos_fwrite(handle: u16, count: u16, buffer: near ^u8): u16
        external reentrant procedure gemdos_cconout(ch: u8)

        procedure main()
            var handle: u16
            var status: u16
            serial_init()
            gemdos_init()
            status = dos_mount_fat_drive(0)
            handle = gemdos_fopen(\"FILE.TXT\", 0)
            write_buffer[0] = 'B'
            write_buffer[1] = 'Y'
            write_buffer[2] = 'E'
            write_buffer[3] = 'L'
            write_buffer[4] = 'O'
            write_buffer[5] = '!'
            written = gemdos_fwrite(handle, 3, WRITE_BUFFER)
            gemdos_cconout('!')
            return
        end
        ",
    )
    .expect("caller source should be written");

    let build = build_mensch_xex_sources(
        "language-gemdos-fat-file-write",
        &[
            caller_path,
            repo_root.join("examples/hellord-dos.mensch"),
            repo_root.join("examples/hellord-bios.mensch"),
            repo_root.join("examples/hellord-gemdos.mensch"),
            repo_root.join("examples/hellord-traps.mensch"),
            repo_root.join("examples/hellord-console.mensch"),
            repo_root.join("examples/hellord-files.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("GEMDOS FAT file write image should build");

    let mut disk = vec![0; 2048];
    disk[11] = 0x00;
    disk[12] = 0x02;
    disk[13] = 1;
    disk[14] = 1;
    disk[16] = 1;
    disk[17] = 16;
    disk[22] = 1;
    let root = 1024;
    disk[root..root + 11].copy_from_slice(b"FILE    TXT");
    disk[root + 11] = 0x20;
    disk[root + 26] = 2;
    disk[root + 28] = 6;
    disk[1536..1542].copy_from_slice(b"HELLO!");

    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 1_000_000,
            stop_after_output_len: Some(1),
            live_input: None,
            block_device: Some(BlockDeviceConfig {
                image: disk,
                read_only: false,
            }),
        },
        &mut Vec::new(),
    )
    .expect("GEMDOS FAT file write image should run");

    let disk = run
        .machine
        .block_device_image()
        .expect("block image should remain attached");
    assert_eq!(run.output, b"!");
    assert_eq!(run.machine.byte(0x000304), 3);
    assert_eq!(run.machine.byte(0x000305), 0);
    assert_eq!(&disk[1536..1542], b"BYELO!");
}

#[test]
fn language_gemdos_console_discovers_bios_device() {
    let dir = temp_dir("language-gemdos-console-device");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let cfg_path = dir.join("image.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::write(&cfg_path, large_integration_xex_linker_config())
        .expect("large integration linker config should be written");

    fs::write(
        &caller_path,
        "
        module GemdosConsoleCaller
        application main
        vector irq = serial_irq
        fixed($0002aa) volatile var stdin_device: u16
        fixed($0002ac) volatile var stdout_device: u16
        fixed($0002ae) volatile var input_status: u16
        fixed($0002b0) volatile var input_value: u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant procedure gemdos_init()
        external reentrant function gemdos_get_stddev(stream: u16): u16
        external reentrant function gemdos_cconis(): u16
        external reentrant function gemdos_cconin(): u8
        external reentrant procedure gemdos_cconout(ch: u8)

        procedure main()
            serial_init()
            gemdos_init()
            stdin_device = gemdos_get_stddev(0)
            stdout_device = gemdos_get_stddev(1)
            input_status = gemdos_cconis()
            input_value = gemdos_cconin()
            gemdos_cconout(input_value)
            return
        end
        ",
    )
    .expect("caller source should be written");

    let build = build_mensch_xex_sources(
        "language-gemdos-console-device",
        &[
            caller_path,
            repo_root.join("examples/hellord-dos.mensch"),
            repo_root.join("examples/hellord-bios.mensch"),
            repo_root.join("examples/hellord-gemdos.mensch"),
            repo_root.join("examples/hellord-traps.mensch"),
            repo_root.join("examples/hellord-console.mensch"),
            repo_root.join("examples/hellord-files.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("GEMDOS console image should build");

    let run = run_image_until_output(&build.image, b"G", 1, 100_000)
        .expect("GEMDOS console image should echo one byte");

    assert_eq!(run.output, b"G");
    assert_eq!(run.machine.byte(0x0002aa), 0x02);
    assert_eq!(run.machine.byte(0x0002ab), 0x00);
    assert_eq!(run.machine.byte(0x0002ac), 0x02);
    assert_eq!(run.machine.byte(0x0002ad), 0x00);
    assert_eq!(run.machine.byte(0x0002ae), 0xff);
    assert_eq!(run.machine.byte(0x0002af), 0xff);
    assert_eq!(run.machine.byte(0x0002b0), b'G');
}

#[test]
fn language_gemdos_drive_syscalls_update_dos_current_drive() {
    let dir = temp_dir("language-gemdos-drive-syscalls");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let cfg_path = dir.join("image.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::write(&cfg_path, large_integration_xex_linker_config())
        .expect("large integration linker config should be written");

    fs::write(
        &caller_path,
        "
        module GemdosDriveCaller
        application main
        vector irq = serial_irq
        fixed($0002b4) volatile var initial_drive: u16
        fixed($0002b6) volatile var set_mask: u16
        fixed($0002b8) volatile var selected_drive: u16
        fixed($0002be) volatile var set_root_status: u16
        fixed($0002c0) volatile var root_first_status: u16
        fixed($0002c2) volatile var root_next_status: u16
        const ROOT_DTA = $9800
        fixed($009800) var root_dta: [32]u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant procedure gemdos_init()
        external reentrant function gemdos_dgetdrv(): u16
        external reentrant function gemdos_dsetdrv(drive: u16): u16
        external reentrant function gemdos_dsetpath(path: near ^u8): u16
        external reentrant function gemdos_fsfirst(pattern: near ^u8, dta: near ^u8): u16
        external reentrant function gemdos_fsnext(dta: near ^u8): u16
        external reentrant procedure gemdos_cconout(ch: u8)

        procedure main()
            serial_init()
            gemdos_init()
            initial_drive = gemdos_dgetdrv()
            set_mask = gemdos_dsetdrv(2)
            selected_drive = gemdos_dgetdrv()
            set_root_status = gemdos_dsetpath(\"/\")
            root_first_status = gemdos_fsfirst(\"*.*\", ROOT_DTA)
            root_next_status = gemdos_fsnext(ROOT_DTA)
            gemdos_cconout('!')
            return
        end
        ",
    )
    .expect("caller source should be written");

    let build = build_mensch_xex_sources(
        "language-gemdos-drive-syscalls",
        &[
            caller_path,
            repo_root.join("examples/hellord-dos.mensch"),
            repo_root.join("examples/hellord-bios.mensch"),
            repo_root.join("examples/hellord-gemdos.mensch"),
            repo_root.join("examples/hellord-traps.mensch"),
            repo_root.join("examples/hellord-console.mensch"),
            repo_root.join("examples/hellord-files.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("GEMDOS drive syscall image should build");

    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 200_000,
            stop_after_output_len: Some(1),
            live_input: None,
            block_device: None,
        },
        &mut Vec::new(),
    )
    .expect("GEMDOS drive syscall image should run");

    assert_eq!(run.output, b"!");
    assert_eq!(run.machine.byte(0x0002b4), 0);
    assert_eq!(run.machine.byte(0x0002b5), 0);
    assert_eq!(run.machine.byte(0x0002b6), 0xff);
    assert_eq!(run.machine.byte(0x0002b7), 0xff);
    assert_eq!(run.machine.byte(0x0002b8), 2);
    assert_eq!(run.machine.byte(0x0002b9), 0);
    assert_eq!(run.machine.byte(0x0002be), 0);
    assert_eq!(run.machine.byte(0x0002bf), 0);
    assert_eq!(run.machine.byte(0x0002c0), 0);
    assert_eq!(run.machine.byte(0x0002c1), 0);
    assert_eq!(run.machine.byte(0x0002c2), 0);
    assert_eq!(run.machine.byte(0x0002c3), 0);
    assert_eq!(run.machine.byte(0x009800), 0);
    assert_eq!(run.machine.byte(0x009801), 65);
    assert_eq!(run.machine.byte(0x009802), 0);
    assert_eq!(run.machine.byte(0x009803), b'S');
    assert_eq!(run.machine.byte(0x009804), b'Y');
    assert_eq!(run.machine.byte(0x009805), b'S');
}

#[test]
fn language_gemdos_file_syscalls_read_romfs_file() {
    let dir = temp_dir("language-gemdos-file-syscalls");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let cfg_path = dir.join("image.cfg");
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::write(&cfg_path, large_integration_xex_linker_config())
        .expect("large integration linker config should be written");

    fs::write(
        &caller_path,
        "
        module GemdosFileCaller
        application main
        vector irq = serial_irq
        fixed($0002d0) volatile var handle: u16
        fixed($0002d2) volatile var read_count: u16
        fixed($0002d6) volatile var write_status: u16
        fixed($0002d8) volatile var close_status: u16
        const READ_BUFFER = $9880
        fixed($009880) var read_buffer: [32]u8

        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant procedure gemdos_init()
        external reentrant function gemdos_fopen(path: near ^u8, mode: u16): u16
        external reentrant function gemdos_fread(handle: u16, count: u16, buffer: near ^u8): u16
        external reentrant function gemdos_fwrite(handle: u16, count: u16, buffer: near ^u8): u16
        external reentrant function gemdos_fclose(handle: u16): u16
        external reentrant procedure gemdos_cconout(ch: u8)

        procedure main()
            serial_init()
            gemdos_init()
            handle = gemdos_fopen(\"SYSTEM.TXT\", 0)
            read_count = gemdos_fread(handle, 12, READ_BUFFER)
            write_status = gemdos_fwrite(handle, 1, READ_BUFFER)
            close_status = gemdos_fclose(handle)
            gemdos_cconout('!')
            return
        end
        ",
    )
    .expect("caller source should be written");

    let build = build_mensch_xex_sources(
        "language-gemdos-file-syscalls",
        &[
            caller_path,
            repo_root.join("examples/hellord-dos.mensch"),
            repo_root.join("examples/hellord-bios.mensch"),
            repo_root.join("examples/hellord-gemdos.mensch"),
            repo_root.join("examples/hellord-traps.mensch"),
            repo_root.join("examples/hellord-console.mensch"),
            repo_root.join("examples/hellord-files.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
        ],
        &cfg_path,
    )
    .expect("GEMDOS file syscall image should build");

    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: Vec::new(),
            max_steps: 400_000,
            stop_after_output_len: Some(1),
            live_input: None,
            block_device: None,
        },
        &mut Vec::new(),
    )
    .expect("GEMDOS file syscall image should run");

    assert_eq!(run.output, b"!");
    assert_eq!(run.machine.byte(0x0002d0), 6);
    assert_eq!(run.machine.byte(0x0002d1), 0);
    assert_eq!(run.machine.byte(0x0002d2), 12);
    assert_eq!(run.machine.byte(0x0002d3), 0);
    assert_eq!(run.machine.byte(0x0002d6), 0xfb);
    assert_eq!(run.machine.byte(0x0002d7), 0xff);
    assert_eq!(run.machine.byte(0x0002d8), 0);
    assert_eq!(run.machine.byte(0x0002d9), 0);
    assert_eq!(
        &[
            run.machine.byte(0x009880),
            run.machine.byte(0x009881),
            run.machine.byte(0x009882),
            run.machine.byte(0x009883),
            run.machine.byte(0x009884),
            run.machine.byte(0x009885),
            run.machine.byte(0x009886),
            run.machine.byte(0x009887),
            run.machine.byte(0x009888),
            run.machine.byte(0x009889),
            run.machine.byte(0x00988a),
            run.machine.byte(0x00988b),
        ],
        b"CPU: 65816\r\n"
    );
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
    machine.load(DEFAULT_ROMBLOCK_NATIVE_VECTOR_PHYSICAL, &vectors);
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
    machine.load(DEFAULT_ROMBLOCK_NATIVE_VECTOR_PHYSICAL, &vectors);
    machine.push_serial_input(b"!");

    let (_cpu, machine) = run_loaded_routine_at(machine, IRQ_ROUTINE_ADDR);

    assert_eq!(machine.byte(0x000292), b'!');
    assert_eq!(machine.serial_output(), b"Z");
}

#[test]
fn language_irq_preserves_interrupted_data_bank_by_default() {
    let (code, vectors) = assemble_source_with_vectors_fixture(
        "language-uart-irq-preserves-dbr",
        &format!(
            "
            module UartIrqPreserveDbrDemo
            vector irq = irq
            fixed(${uart_ctrl:06x}) volatile var uart_ctrl: u8
            fixed(${serial_data:06x}) volatile var serial_data: u8
            fixed($000294) volatile var seen: u8

            export far procedure main()
                asm \"sei\"
                uart_ctrl = ${rx_irq:02x}
                asm \"sep #$20\"
                asm \"lda #$01\"
                asm \"pha\"
                asm \"plb\"
                asm \"lda #$7e\"
                asm \"cli\"
                asm \"nop\"
                asm \"sta $9000\"
                return
            end

            interrupt procedure irq()
                seen = serial_data
                asm \"lda #$00\"
                asm \"pha\"
                asm \"plb\"
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
    machine.load(DEFAULT_ROMBLOCK_NATIVE_VECTOR_PHYSICAL, &vectors);
    machine.push_serial_input(b"!");

    let (_cpu, machine) = run_loaded_routine_at(machine, IRQ_ROUTINE_ADDR);

    assert_eq!(machine.byte(0x000294), b'!');
    assert_eq!(machine.byte(0x019000), 0x7e);
    assert_ne!(machine.byte(0x009000), 0x7e);
}

#[test]
fn language_irq_preserve_none_can_clobber_interrupted_accumulator() {
    let (code, vectors) = assemble_source_with_vectors_fixture(
        "language-uart-irq-preserve-none-clobbers-a",
        &format!(
            "
            module UartIrqPreserveNoneDemo
            vector irq = irq
            fixed(${uart_ctrl:06x}) volatile var uart_ctrl: u8
            fixed(${serial_data:06x}) volatile var serial_data: u8
            fixed($000296) volatile var seen: u8

            export far procedure main()
                asm \"sei\"
                uart_ctrl = ${rx_irq:02x}
                asm \"lda #$5a\"
                asm \"cli\"
                asm \"nop\"
                asm \"sta ${serial_data:06x}\"
                return
            end

            interrupt(preserve = none) procedure irq()
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
    machine.load(DEFAULT_ROMBLOCK_NATIVE_VECTOR_PHYSICAL, &vectors);
    machine.push_serial_input(b"!");

    let (_cpu, machine) = run_loaded_routine_at(machine, IRQ_ROUTINE_ADDR);

    assert_eq!(machine.byte(0x000296), b'!');
    assert_eq!(machine.serial_output(), b"!");
}

#[test]
fn raw_interrupt_skips_generated_entry_state() {
    let (code, vectors) = assemble_source_with_vectors_fixture(
        "language-uart-raw-irq-skips-entry-state",
        &format!(
            "
            module RawIrqEntryDemo
            vector irq = irq
            fixed(${uart_ctrl:06x}) volatile var uart_ctrl: u8
            fixed($000298) volatile var seen: u8

            export far procedure main()
                asm \"sei\"
                uart_ctrl = ${rx_irq:02x}
                asm \"sep #$20\"
                asm \"lda #$01\"
                asm \"pha\"
                asm \"plb\"
                asm \"cli\"
                asm \"nop\"
                return
            end

            interrupt(raw) procedure irq()
                asm \"sep #$20\"
                asm \".a8\"
                asm \"lda #$42\"
                asm \"sta $9000\"
                asm \"lda f:${serial_data:06x}\"
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
    machine.load(DEFAULT_ROMBLOCK_NATIVE_VECTOR_PHYSICAL, &vectors);
    machine.push_serial_input(b"!");

    let (_cpu, machine) = run_loaded_routine_at(machine, IRQ_ROUTINE_ADDR);

    assert_eq!(machine.byte(0x019000), 0x42);
    assert_ne!(machine.byte(0x009000), 0x42);
    assert_eq!(machine.byte(0x000298), 0);
}

#[test]
fn hellord_example_boots_as_interrupt_driven_xex() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> HELP\r\nCommands:\r\nH  help\r\nV  version\r\nD  dir\r\nT  type\r\nR  run\r\nE  echo\r\nS  status\r\nP  pwd\r\nM  map\r\nMEM status\r\nBLKINFO disk\r\nC  cls\r\nPATH mode\r\nSET name value\r\nPROMPT mode\r\nCD path\r\nPEEK addr\r\nPOKE addr byte\r\nDUMP addr len\r\nFILL addr len byte\r\nA:-Z: drive\r\nCtrl-P recall\r\nA> VER\r\nHellord DOS 0.1\r\nA> DIR\r\nDirectory of A:/\r\nREADME.TXT  $0031\r\nSYSTEM.TXT  $0041\r\nHELLO.PRG  $0025\r\nGAMES      <DIR>\r\n3 file(s)  1 dir(s)\r\nA> b:\r\nB> DIR\r\nDirectory of B:/\r\nREADME.TXT  $0031\r\nSYSTEM.TXT  $0041\r\nHELLO.PRG  $0025\r\nGAMES      <DIR>\r\n3 file(s)  1 dir(s)\r\nB> PWD\r\nB:/\r\nB> CD GAMES\r\nB> PWD\r\nB:GAMES\r\nB> MAP\r\nMemory map:\r\nRAM $0000-$8FFF\r\nI/O $9F00-$9FFF\r\nROM $C000-$FFFF\r\nB> STAT\r\nR: 0\r\nT: 0\r\nB> ECHO heX\x08 \x08llo\r\nhello\r\nB> X\r\n?SYNTAX\r\nB> \r\nB> ";

    let run = run_image_until_output(
        &build.image,
        b"HELP\nVER\nDIR\nb:\nDIR\nPWD\nCD GAMES\nPWD\nMAP\nSTAT\nECHO heX\x08llo\nX\n\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should produce expected output");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_blkinfo_reads_attached_block_sector_through_bios() {
    let build = build_hellord_example();
    let mut disk = vec![0; 1024];
    disk[0..15].copy_from_slice(b"MENSCH BLOCKDEV");
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> blkinfo\r\nDrive A block:\r\nStatus $03\r\nSectors $0002\r\nSector0: 4D 45 4E 53 43 48 20 42 4C 4F 43 4B 44 45 56 00\r\nA> ";
    let mut output = Vec::new();
    let run = x65_runner::run_image(
        &build.image,
        RunConfig {
            serial_input: b"blkinfo\n".to_vec(),
            max_steps: 1_000_000,
            stop_after_output_len: Some(expected.len()),
            live_input: None,
            block_device: Some(BlockDeviceConfig {
                image: disk,
                read_only: false,
            }),
        },
        &mut output,
    )
    .expect("Hellord image should read sector zero through BIOS block calls");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_blkinfo_reports_romfs_drives_without_block_mapping() {
    let build = build_hellord_example();
    let expected =
        b"Hellord DOS 0.1\r\nType H for help.\r\nA> b:\r\nB> blkinfo\r\nDrive B block:\r\nNo block device\r\nB> ";
    let run = run_image_until_output(&build.image, b"b:\nblkinfo\n", expected.len(), 1_000_000)
        .expect("Hellord image should report that ROMFS-only drives have no block device");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_set_lists_and_updates_environment_state() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> set\r\nDRIVE=A\r\nDIR=/\r\nPROMPT=SHORT\r\nPATH=.\r\nA> set prompt full\r\nA:/> set drive c\r\nC:/> set\r\nDRIVE=C\r\nDIR=/\r\nPROMPT=FULL\r\nPATH=.\r\nC:/> set prompt short\r\nC> ";

    let run = run_image_until_output(
        &build.image,
        b"set\nset prompt full\nset drive c\nset\nset prompt short\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should produce SET environment transcript");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_mem_reports_runtime_memory_status() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> mem\r\nMemory status:\r\nDirect $0080-$00FF\r\nBSS $9000-$9EFF\r\nLine buffer $0020\r\nSerial rings $0040\r\nA> ";

    let run = run_image_until_output(&build.image, b"mem\n", expected.len(), 1_000_000)
        .expect("Hellord image should produce MEM status transcript");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_set_rejects_invalid_assignments() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> set drive 1\r\n?SYNTAX\r\nA> ";

    let run = run_image_until_output(&build.image, b"set drive 1\n", expected.len(), 1_000_000)
        .expect("Hellord image should reject bad SET assignment");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_path_controls_bare_program_search() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> cd games\r\nA> hello\r\n?SYNTAX\r\nA> path\r\nPATH=.\r\nA> path /\r\nA> hello\r\nHELLO.PRG says: Hello from ROM.\r\nA> set path .\r\nA> hello\r\n?SYNTAX\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"cd games\nhello\npath\npath /\nhello\nset path .\nhello\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should produce PATH search transcript");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_prompt_modes_and_clear_screen_work() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> prompt\r\nPrompt: SHORT\r\nA> prompt full\r\nA:/> cd games\r\nA:GAMES> prompt\r\nPrompt: FULL\r\nA:GAMES> cls\r\n\x1b[2J\x1b[HA:GAMES> prompt short\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"prompt\nprompt full\ncd games\nprompt\ncls\nprompt short\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should produce prompt settings transcript");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_rom_files_can_be_listed_and_typed() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> dir\r\nDirectory of A:/\r\nREADME.TXT  $0031\r\nSYSTEM.TXT  $0041\r\nHELLO.PRG  $0025\r\nGAMES      <DIR>\r\n3 file(s)  1 dir(s)\r\nA> t readme\r\nHellord is a tiny Mensch DOS for the x65-alike.\r\nA> type SYSTEM.TXT\r\nCPU: 65816\r\nConsole: USB UART IRQ rings\r\nStorage: ROM file table\r\nA> t missing\r\nFile not found\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"dir\nt readme\ntype SYSTEM.TXT\nt missing\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should produce ROM file transcript");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_rom_programs_can_be_run_by_directory() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> run hello\r\nHELLO.PRG says: Hello from ROM.\r\nA> hello\r\nHELLO.PRG says: Hello from ROM.\r\nA> run missing\r\nProgram not found\r\nA> cd games\r\nA> r demo\r\nDEMO.PRG says: sprites someday.\r\nA> demo\r\nDEMO.PRG says: sprites someday.\r\nA> hello\r\n?SYNTAX\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"run hello\nhello\nrun missing\ncd games\nr demo\ndemo\nhello\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should run ROM program transcript");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_cd_uses_rom_directory_namespace() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> cd missing\r\nPath not found\r\nA> cd games\r\nA> dir\r\nDirectory of A:GAMES\r\nDEMO.TXT  $0030\r\nDEMO.PRG  $0024\r\n2 file(s)  0 dir(s)\r\nA> type demo\r\nNo games yet, but the directory layer works.\r\nA> cd ..\r\nA> pwd\r\nA:/\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"cd missing\ncd games\ndir\ntype demo\ncd ..\npwd\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should produce directory namespace transcript");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_command_matcher_handles_aliases_and_rejects_partial_words() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> p\r\nA:/\r\nA> cd \r\n?SYNTAX\r\nA> e hi\r\nhi\r\nA> echox\r\n?SYNTAX\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"p\ncd \ne hi\nechox\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should produce matcher transcript");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_recalls_last_command_with_ctrl_p() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> echo recalled\r\nrecalled\r\nA> echo recalled\r\nrecalled\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"echo recalled\n\x10\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should recall the previous line with Ctrl-P");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_peeks_and_pokes_ram_bytes() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> poke 400 2a\r\nOK\r\nA> peek 400\r\n$0400 = $2A\r\nA> dump 400 4\r\n$0400: 2A 00 00 00\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"poke 400 2a\npeek 400\ndump 400 4\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should inspect and modify RAM bytes");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(0x000400), 0x2a);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_monitor_commands_reject_bad_hex_arguments() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> dump 400 11\r\n?SYNTAX\r\nA> poke 400 100\r\n?SYNTAX\r\nA> peek nope\r\n?SYNTAX\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"dump 400 11\npoke 400 100\npeek nope\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should reject bad monitor arguments");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_fills_ram_ranges() {
    let build = build_hellord_example();
    let expected = b"Hellord DOS 0.1\r\nType H for help.\r\nA> fill 410 4 ff\r\nOK\r\nA> dump 410 4\r\n$0410: FF FF FF FF\r\nA> fill 410 0 ff\r\n?SYNTAX\r\nA> ";

    let run = run_image_until_output(
        &build.image,
        b"fill 410 4 ff\ndump 410 4\nfill 410 0 ff\n",
        expected.len(),
        1_000_000,
    )
    .expect("Hellord image should fill RAM ranges");

    assert_eq!(run.output, expected);
    assert_eq!(run.machine.byte(0x000410), 0xff);
    assert_eq!(run.machine.byte(0x000411), 0xff);
    assert_eq!(run.machine.byte(0x000412), 0xff);
    assert_eq!(run.machine.byte(0x000413), 0xff);
    assert_eq!(run.machine.byte(HELLORD_OVERFLOW_ADDR), 0);
    assert_eq!(run.machine.byte(HELLORD_TX_OVERFLOW_ADDR), 0);
}

#[test]
fn hellord_example_flags_full_rx_ring_and_drops_input() {
    let build = build_hellord_example();
    let input = b"abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789overflow";
    let output_len = b"Hellord DOS 0.1\r\nType H for help.\r\nA> ".len() + 8;

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
        external procedure serial_write(value: u16)

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
    build_mensch_x65_image_sources(
        "hellord-os",
        &[
            repo_root.join("examples/hellord-os.mensch"),
            repo_root.join("examples/hellord-dos.mensch"),
            repo_root.join("examples/hellord-console.mensch"),
            repo_root.join("examples/hellord-files.mensch"),
            repo_root.join("examples/serial-ring.mensch"),
            repo_root.join("examples/hellord-bios.mensch"),
            repo_root.join("examples/hellord-gemdos.mensch"),
            repo_root.join("examples/hellord-traps.mensch"),
        ],
        repo_root.join("examples/hellord-os-x65.cfg"),
        CpuStart {
            pbr: 0x00,
            pc: 0xa000,
        },
        &[
            X65ImageBlock {
                address: 0x00a000,
                path: "code.bin".into(),
            },
            X65ImageBlock {
                address: 0x01c000,
                path: "bank1.bin".into(),
            },
            X65ImageBlock {
                address: 0x009f00,
                path: "meta.bin".into(),
            },
            X65ImageBlock {
                address: 0x00ffe4,
                path: "vectors.bin".into(),
            },
        ],
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
fn cross_module_scratch_spills_to_stack_when_direct_page_budget_is_full() {
    let dir = temp_dir("cross-module-scratch-budget");
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let caller_path = dir.join("caller.mensch");
    let callee_path = dir.join("callee.mensch");
    let cfg_path = dir.join("image.cfg");

    let mut caller = String::from(
        "
        module Caller
        application main
        external procedure use_scratch()

        procedure main()
        ",
    );
    for index in 0..storage::DEFAULT_DIRECT_PAGE_BUDGET {
        caller.push_str(&format!("            direct var zp{index}: u8\n"));
    }
    caller.push_str(
        "
            use_scratch()
            asm \"stp\"
            return
        end
        ",
    );
    fs::write(&caller_path, caller).expect("caller source should be written");
    fs::write(
        &callee_path,
        "
        module Callee
        fixed($000284) volatile var out: u8

        export procedure use_scratch()
            scratch var value: u8
            value = $5a
            out = value
            return
        end
        ",
    )
    .expect("callee source should be written");
    fs::write(&cfg_path, xex_linker_config()).expect("linker config should be written");

    let build = build_mensch_xex_sources(
        "cross-module-scratch-budget",
        &[caller_path, callee_path],
        &cfg_path,
    )
    .expect("scratch spill image should build inside the ZP budget");
    let mut output = Vec::new();
    let run = x65_runner::run_image(
        &build.image,
        x65_runner::RunConfig::scripted(b"", 20_000),
        &mut output,
    )
    .expect("scratch spill image should run");

    assert_eq!(run.machine.byte(0x000284), 0x5a);
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
    "MEMORY {\n  ZP: start = $000080, size = $000080, type = rw;\n  CODE: start = $008000, size = $001000, file = %O;\n  META: start = $008f00, size = $000100, file = %O;\n  BSS: start = $009000, size = $000f00, type = rw;\n  VECTORS: start = $00ffe4, size = $00000c, file = %O;\n}\nFILES {\n  %O: format = atari;\n}\nFORMATS {\n  atari: runad = main;\n}\nSEGMENTS {\n  ZEROPAGE: load = ZP, type = zp;\n  CODE: load = CODE, type = ro;\n  MENSCH_META: load = META, type = ro, optional = yes;\n  BSS: load = BSS, type = bss;\n  VECTOR_COP: load = VECTORS, type = ro, optional = yes, offset = $0000;\n  VECTOR_BRK: load = VECTORS, type = ro, optional = yes, offset = $0002;\n  VECTOR_ABORT: load = VECTORS, type = ro, optional = yes, offset = $0004;\n  VECTOR_NMI: load = VECTORS, type = ro, optional = yes, offset = $0006;\n  VECTOR_IRQ: load = VECTORS, type = ro, optional = yes, offset = $000a;\n  VECTORS: load = VECTORS, type = ro, optional = yes;\n}\n"
}

fn large_integration_xex_linker_config() -> &'static str {
    "MEMORY {\n  ZP: start = $000080, size = $000080, type = rw;\n  CODE: start = $002000, size = $007000, file = %O;\n  META: start = $009f00, size = $000100, file = %O;\n  BSS: start = $009000, size = $000f00, type = rw;\n  BANK1: start = $00c000, size = $003fe4, file = %O;\n  VECTORS: start = $00ffe4, size = $00000c, file = %O;\n}\nFILES {\n  %O: format = atari;\n}\nFORMATS {\n  atari: runad = main;\n}\nSEGMENTS {\n  ZEROPAGE: load = ZP, type = zp;\n  CODE: load = CODE, type = ro;\n  MENSCH_META: load = META, type = ro, optional = yes;\n  BSS: load = BSS, type = bss;\n  BANK1: load = BANK1, type = ro;\n  VECTOR_COP: load = VECTORS, type = ro, optional = yes, offset = $0000;\n  VECTOR_BRK: load = VECTORS, type = ro, optional = yes, offset = $0002;\n  VECTOR_ABORT: load = VECTORS, type = ro, optional = yes, offset = $0004;\n  VECTOR_NMI: load = VECTORS, type = ro, optional = yes, offset = $0006;\n  VECTOR_IRQ: load = VECTORS, type = ro, optional = yes, offset = $000a;\n  VECTORS: load = VECTORS, type = ro, optional = yes;\n}\n"
}

fn cop_xex_linker_config() -> &'static str {
    "MEMORY {\n  ZP: start = $000080, size = $000080, type = rw;\n  CODE: start = $008000, size = $002000, file = %O;\n  META: start = $00a000, size = $000100, file = %O;\n  BSS: start = $00a100, size = $000f00, type = rw;\n  VECTORS: start = $00ffe4, size = $00000c, file = %O;\n}\nFILES {\n  %O: format = atari;\n}\nFORMATS {\n  atari: runad = main;\n}\nSEGMENTS {\n  ZEROPAGE: load = ZP, type = zp;\n  CODE: load = CODE, type = ro;\n  MENSCH_META: load = META, type = ro, optional = yes;\n  BSS: load = BSS, type = bss;\n  VECTOR_COP: load = VECTORS, type = ro, optional = yes, offset = $0000;\n  VECTOR_BRK: load = VECTORS, type = ro, optional = yes, offset = $0002;\n  VECTOR_ABORT: load = VECTORS, type = ro, optional = yes, offset = $0004;\n  VECTOR_NMI: load = VECTORS, type = ro, optional = yes, offset = $0006;\n  VECTOR_IRQ: load = VECTORS, type = ro, optional = yes, offset = $000a;\n  VECTORS: load = VECTORS, type = ro, optional = yes;\n}\n"
}

fn bios_only_traps_source() -> &'static str {
    "
    module BiosOnlyTraps

    vector cop = cop_dispatch

    external reentrant function bios_trap_dispatch(call: u16, arg0: u16, arg1: u16): u16

    const COP_FAMILY_BIOS = $0d

    interrupt(cop, preserve = x, y, p, dbr) function cop_dispatch(): u16
        if cop_signature() == COP_FAMILY_BIOS
            return bios_trap_dispatch(cop_a(), cop_x(), cop_y())
        else
            return $ffff
        end
    end
    "
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
