use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mensch_systems_tests::{ca65, fixtures, frontend, lower_65816};
use wdc65816::{HasAddressBus, Processor, StatusRegister};

const ROUTINE_ADDR: usize = 0x020000;
const RETURN_BANK: u8 = 0x03;
const RETURN_ADDR: u16 = 0x1234;

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
    run_loaded_generated_routine(memory)
}

fn run_loaded_generated_routine(mut memory: Memory) -> (Processor, Memory) {
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

fn assemble_ir_fixture(name: &str, module: &mensch_systems_tests::ir::Module) -> Vec<u8> {
    let asm = ca65::emit_module(&lower_65816::lower_module(module).unwrap());
    assemble_fixture(name, &asm)
}

fn assemble_storage_fixture(name: &str, module: &mensch_systems_tests::ir::Module) -> Vec<u8> {
    let (lowered, _assignment) = lower_65816::lower_module_with_storage(module)
        .expect("fixture should assign storage and lower to 65816 assembly");
    let asm = ca65::emit_module(&lowered);
    assemble_fixture(name, &asm)
}

fn assemble_source_fixture(name: &str, source: &str) -> Vec<u8> {
    let module = frontend::compile_source(source).expect("source fixture should compile to IR");
    assemble_storage_fixture(name, &module)
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
    "MEMORY {\n  ZP: start = $000080, size = $000080, type = rw;\n  CODE: start = $020000, size = $001000, file = %O;\n  META: start = $020f00, size = $000100, file = %O;\n  BSS: start = $021000, size = $001000, type = rw;\n}\nSEGMENTS {\n  ZEROPAGE: load = ZP, type = zp;\n  CODE: load = CODE, type = ro;\n  MENSCH_META: load = META, type = ro;\n  BSS: load = BSS, type = bss;\n}\n"
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
    let binary = assemble_ir_fixture("return-constant", &fixtures::return_constant_u16());

    let (cpu, _memory) = run_generated_routine(&binary);

    assert_eq!(cpu.c_true(), 0x1234);
}

#[test]
fn generated_store_byte_routine_runs() {
    let binary = assemble_ir_fixture("store-byte", &fixtures::store_byte());

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(fixtures::STORE_BYTE_ADDR as usize), 0x7f);
}

#[test]
fn generated_copy_bytes_near_routine_runs() {
    let binary = assemble_ir_fixture("copy-bytes-near", &fixtures::copy_bytes_near());
    let mut memory = Memory::new();
    memory.load(ROUTINE_ADDR, &binary);
    memory.load(
        0x020000 + fixtures::COPY_SRC as usize,
        &[0x10, 0x20, 0x30, 0x40],
    );

    let (_cpu, memory) = run_loaded_generated_routine(memory);

    assert_eq!(memory.byte(0x020000 + fixtures::COPY_DST as usize), 0x10);
    assert_eq!(
        memory.byte(0x020000 + fixtures::COPY_DST as usize + 1),
        0x20
    );
    assert_eq!(
        memory.byte(0x020000 + fixtures::COPY_DST as usize + 2),
        0x30
    );
    assert_eq!(
        memory.byte(0x020000 + fixtures::COPY_DST as usize + 3),
        0x40
    );
}

#[test]
fn generated_clear_memory_routine_runs() {
    let binary = assemble_ir_fixture("clear-memory", &fixtures::clear_memory());
    let mut memory = Memory::new();
    memory.load(ROUTINE_ADDR, &binary);
    memory.load(
        0x020000 + fixtures::CLEAR_DST as usize,
        &[0xaa, 0xbb, 0xcc, 0xdd],
    );

    let (_cpu, memory) = run_loaded_generated_routine(memory);

    assert_eq!(memory.byte(0x020000 + fixtures::CLEAR_DST as usize), 0x00);
    assert_eq!(
        memory.byte(0x020000 + fixtures::CLEAR_DST as usize + 1),
        0x00
    );
    assert_eq!(
        memory.byte(0x020000 + fixtures::CLEAR_DST as usize + 2),
        0x00
    );
    assert_eq!(
        memory.byte(0x020000 + fixtures::CLEAR_DST as usize + 3),
        0x00
    );
}

#[test]
fn generated_internal_procedure_call_runs() {
    let binary = assemble_ir_fixture(
        "call-internal-procedure",
        &fixtures::call_internal_procedure(),
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(fixtures::HELPER_STORE_ADDR as usize), 0x42);
}

#[test]
fn generated_internal_function_call_runs() {
    let binary = assemble_ir_fixture(
        "call-internal-function",
        &fixtures::call_internal_function(),
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(fixtures::FUNCTION_STORE_ADDR as usize), 0x55);
}

#[test]
fn generated_exported_main_routine_runs() {
    let binary = assemble_ir_fixture("exported-main", &fixtures::exported_main());

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(fixtures::MAIN_STORE_ADDR as usize), 0x99);
}

#[test]
fn generated_direct_local_storage_routine_runs() {
    let binary = assemble_storage_fixture("direct-local-store", &fixtures::direct_local_store());

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(fixtures::DIRECT_STORE_ADDR as usize), 0x66);
}

#[test]
fn generated_frontend_program_runs() {
    let binary = assemble_source_fixture(
        "frontend-program",
        "
        module FrontendDemo

        export far procedure main()
            direct var p: near ^u8
            var x: u8

            p = $1300
            x = one()
            if x == 1
                x = x + 1
            end
            while x != 4
                x = x + 1
            end
            p^ = x
            return
        end

        function one(): u8
            return 1
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x020000 + 0x1300), 4);
}

#[test]
fn generated_frontend_fixed_volatile_io_program_runs() {
    let binary = assemble_source_fixture(
        "frontend-fixed-volatile",
        "
        module FixedVolatileDemo
        fixed($040005) volatile var io: u8

        export far procedure main()
            asm \"nop\"
            io = $5a
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040005), 0x5a);
}

#[test]
fn generated_frontend_relational_conditions_run() {
    let binary = assemble_source_fixture(
        "frontend-relational-conditions",
        "
        module RelationalConditionsDemo
        fixed($040006) volatile var byte_out: u8
        fixed($040008) volatile var word_out: u16

        export far procedure main()
            var x: u8
            var y: u16
            x = 3
            y = 1000
            if x < 4
                byte_out = byte_out + 1
            end
            if x <= 3
                byte_out = byte_out + 2
            end
            if y > 999
                word_out = word_out + 4
            end
            if y >= 1000
                word_out = word_out + 8
            end
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040006), 3);
    assert_eq!(memory.byte(0x040008), 12);
    assert_eq!(memory.byte(0x040009), 0);
}

#[test]
fn generated_indexed_assignment_from_indexed_value_runs() {
    let binary = assemble_source_fixture(
        "frontend-indexed-assignment-from-indexed-value",
        "
        module IndexedAssignmentDemo
        fixed($04000a) volatile var byte_out: u8
        var source: [4]u8
        var target: [4]u8

        export far procedure main()
            var src: u16
            var dst: u16
            src = 2
            dst = 1
            source[src] = 'Z'
            target[dst] = source[src]
            byte_out = target[dst]
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x04000a), b'Z');
}

#[test]
fn generated_frontend_parameter_call_runs() {
    let binary = assemble_source_fixture(
        "frontend-parameter-call",
        "
        module ParamCallDemo
        fixed($040006) volatile var io: u8

        export far procedure main()
            store('P')
            return
        end

        procedure store(value: u8)
            io = value
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040006), b'P');
}

#[test]
fn generated_frontend_multi_parameter_call_runs() {
    let binary = assemble_source_fixture(
        "frontend-multi-parameter-call",
        "
        module MultiParamCallDemo
        fixed($040006) volatile var byte_out: u8
        fixed($040008) volatile var word_out: u16

        export far procedure main()
            store_pair('Q', $1234)
            return
        end

        procedure store_pair(first: u8, second: u16)
            byte_out = first
            word_out = second
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040006), b'Q');
    assert_eq!(memory.byte(0x040008), 0x34);
    assert_eq!(memory.byte(0x040009), 0x12);
}

#[test]
fn generated_frontend_word_parameter_forwarding_restores_accumulator_width() {
    let binary = assemble_source_fixture(
        "frontend-word-parameter-forwarding-width",
        "
        module WordParamForwardingDemo
        fixed($040006) volatile var byte_out: u8
        fixed($040008) volatile var word_out: u16

        export far procedure main()
            store_source($8fff)
            return
        end

        procedure store_source(value: u16)
            force_byte('!')
            store_word(value)
            return
        end

        procedure force_byte(value: u8)
            byte_out = value
            return
        end

        procedure store_word(value: u16)
            word_out = value
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040006), b'!');
    assert_eq!(memory.byte(0x040008), 0xff);
    assert_eq!(memory.byte(0x040009), 0x8f);
}

#[test]
fn generated_frontend_reentrant_stack_parameter_call_runs() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-stack-parameter-call",
        "
        module ReentrantParamCallDemo
        fixed($04000a) volatile var byte_out: u8

        export far procedure main()
            add('A', 2)
            return
        end

        reentrant procedure add(a: u8, b: u8)
            byte_out = a + b
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x04000a), b'C');
}

#[test]
fn generated_frontend_reentrant_stack_word_parameter_call_runs() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-stack-word-parameter-call",
        "
        module ReentrantWordParamCallDemo
        fixed($04000a) volatile var byte_out: u8
        fixed($04000c) volatile var word_out: u16

        export far procedure main()
            store_pair('Z', $4567)
            return
        end

        reentrant procedure store_pair(a: u8, b: u16)
            byte_out = a
            word_out = b
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x04000a), b'Z');
    assert_eq!(memory.byte(0x04000c), 0x67);
    assert_eq!(memory.byte(0x04000d), 0x45);
}

#[test]
fn generated_frontend_reentrant_stack_local_call_runs() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-stack-local-call",
        "
        module ReentrantLocalCallDemo
        fixed($04000e) volatile var byte_out: u8

        export far procedure main()
            work('A')
            byte_out = byte_out + 1
            return
        end

        reentrant procedure work(a: u8)
            var tmp: u8
            tmp = a + 1
            byte_out = tmp
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x04000e), b'C');
}

#[test]
fn generated_frontend_application_result_program_runs() {
    let binary = assemble_source_fixture(
        "frontend-application-result",
        "
        module AppDemo
        import Bytes as B
        library strings
        application main
        arena var heap: [16]u8
        fixed($040006) volatile var status_word: result(u8, u8)

        procedure main()
            status_word = status()
            return
        end

        function status(): result(u8, u8)
            return ok($2a)
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040006), 0x00);
    assert_eq!(memory.byte(0x040007), 0x2a);
}
