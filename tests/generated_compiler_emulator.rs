use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mensch_systems_tests::{ca65, fixtures};
use wdc65816::{HasAddressBus, Processor, StatusRegister};

const ROUTINE_ADDR: usize = 0x020000;
const RETURN_BANK: u8 = 0x03;
const RETURN_ADDR: u16 = 0x1234;
const STORE_ADDR: usize = 0x040000;

static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
struct Memory {
    bytes: Vec<u8>,
}

impl Memory {
    fn new() -> Self {
        Self {
            bytes: vec![0; 0x1_000000],
        }
    }

    fn load(&mut self, addr: usize, data: &[u8]) {
        self.bytes[addr..addr + data.len()].copy_from_slice(data);
    }

    fn byte(&self, addr: usize) -> u8 {
        self.bytes[addr]
    }
}

impl HasAddressBus for Memory {
    fn read(&mut self, address: usize) -> u8 {
        self.bytes[address & 0x00ff_ffff]
    }

    fn write(&mut self, address: usize, value: u8) {
        self.bytes[address & 0x00ff_ffff] = value;
    }

    fn io(&mut self) {}
}

fn run_generated_routine(binary: &[u8]) -> (Processor, Memory) {
    let mut memory = Memory::new();
    memory.load(ROUTINE_ADDR, binary);

    let mut cpu = Processor::new();
    cpu.p = StatusRegister::from_byte(0x00, false);
    cpu.pbr = (ROUTINE_ADDR >> 16) as u8;
    cpu.dbr = (ROUTINE_ADDR >> 16) as u8;
    cpu.pc = ROUTINE_ADDR as u16;
    cpu.s = 0x01fc;

    let return_minus_one = RETURN_ADDR.wrapping_sub(1).to_le_bytes();
    memory.write(0x01fd, return_minus_one[0]);
    memory.write(0x01fe, return_minus_one[1]);
    memory.write(0x01ff, RETURN_BANK);

    for _ in 0..1_000 {
        if cpu.pbr == RETURN_BANK && cpu.pc == RETURN_ADDR {
            return (cpu, memory);
        }
        cpu.step(&mut memory);
    }

    panic!("generated routine did not return; cpu={cpu:?}");
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
    "MEMORY {\n  CODE: start = $020000, size = $010000, file = %O;\n}\nSEGMENTS {\n  CODE: load = CODE, type = ro;\n}\n"
}

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mensch-generated-{name}-{}-{nanos}-{id}",
        std::process::id()
    ))
}

#[test]
fn generated_return_constant_routine_runs() {
    let asm = ca65::emit_module(&fixtures::return_constant_u16());
    let binary = assemble_fixture("return-constant", &asm);

    let (cpu, _memory) = run_generated_routine(&binary);

    assert_eq!(cpu.c_true(), 0x1234);
}

#[test]
fn generated_store_byte_routine_runs() {
    let asm = ca65::emit_module(&fixtures::store_byte());
    let binary = assemble_fixture("store-byte", &asm);

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(STORE_ADDR), 0x7f);
}
