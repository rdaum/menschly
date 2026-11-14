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

fn run_generated_near_routine(binary: &[u8]) -> (Processor, Memory) {
    let mut memory = Memory::new();
    memory.load(ROUTINE_ADDR, binary);
    run_loaded_generated_near_routine(memory)
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

    run_until_return(cpu, memory, 1_000)
}

fn run_until_return(mut cpu: Processor, mut memory: Memory, steps: usize) -> (Processor, Memory) {
    for _ in 0..steps {
        if cpu.pbr == RETURN_BANK && cpu.pc == RETURN_ADDR {
            return (cpu, memory);
        }
        cpu.step(&mut memory);
    }

    panic!("generated routine did not return; cpu={cpu:?}");
}

/// Like [`run_generated_routine`], for routines that run long loops.
fn run_long_generated_routine(binary: &[u8]) -> (Processor, Memory) {
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
    run_until_return(cpu, memory, 200_000)
}

fn run_loaded_generated_near_routine(mut memory: Memory) -> (Processor, Memory) {
    let mut cpu = Processor::new();
    cpu.p = StatusRegister::from_byte(0x00, false);
    cpu.pbr = (ROUTINE_ADDR >> 16) as u8;
    cpu.dbr = (ROUTINE_ADDR >> 16) as u8;
    cpu.pc = ROUTINE_ADDR as u16;
    cpu.s = 0x01fc;

    let return_minus_one = RETURN_ADDR.wrapping_sub(1).to_le_bytes();
    memory.write(0x01fd, return_minus_one[0]);
    memory.write(0x01fe, return_minus_one[1]);

    for _ in 0..1_000 {
        if cpu.pbr == (ROUTINE_ADDR >> 16) as u8 && cpu.pc == RETURN_ADDR {
            return (cpu, memory);
        }
        cpu.step(&mut memory);
    }

    panic!("generated near routine did not return; cpu={cpu:?}");
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

    let (cpu, _memory) = run_generated_near_routine(&binary);

    assert_eq!(cpu.c_true(), 0x1234);
}

#[test]
fn generated_store_byte_routine_runs() {
    let binary = assemble_ir_fixture("store-byte", &fixtures::store_byte());

    let (_cpu, memory) = run_generated_near_routine(&binary);

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

    let (_cpu, memory) = run_loaded_generated_near_routine(memory);

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

    let (_cpu, memory) = run_loaded_generated_near_routine(memory);

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

    let (_cpu, memory) = run_generated_near_routine(&binary);

    assert_eq!(memory.byte(fixtures::HELPER_STORE_ADDR as usize), 0x42);
}

#[test]
fn generated_internal_function_call_runs() {
    let binary = assemble_ir_fixture(
        "call-internal-function",
        &fixtures::call_internal_function(),
    );

    let (_cpu, memory) = run_generated_near_routine(&binary);

    assert_eq!(memory.byte(fixtures::FUNCTION_STORE_ADDR as usize), 0x55);
}

#[test]
fn generated_exported_main_routine_runs() {
    let binary = assemble_ir_fixture("exported-main", &fixtures::exported_main());

    let (_cpu, memory) = run_generated_near_routine(&binary);

    assert_eq!(memory.byte(fixtures::MAIN_STORE_ADDR as usize), 0x99);
}

#[test]
fn generated_direct_local_storage_routine_runs() {
    let binary = assemble_storage_fixture("direct-local-store", &fixtures::direct_local_store());

    let (_cpu, memory) = run_generated_near_routine(&binary);

    assert_eq!(memory.byte(fixtures::DIRECT_STORE_ADDR as usize), 0x66);
}

#[test]
fn generated_frontend_scratch_local_program_runs() {
    let binary = assemble_source_fixture(
        "frontend-scratch-local",
        "
        module ScratchLocalDemo

        fixed($1300) volatile var out: u8

        export far procedure main()
            scratch var first: u8
            scratch var second: u8

            first = $12
            out = first
            second = $34
            out = second
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x020000 + 0x1300), 0x34);
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
fn generated_frontend_global_pointer_deref_uses_direct_scratch() {
    let binary = assemble_source_fixture(
        "frontend-global-pointer-deref",
        "
        module GlobalPointerDerefDemo

        var p: near ^u8
        fixed($040006) volatile var out: u8

        export far procedure main()
            p = $4006
            p^ = $7b
            out = p^
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040006), 0x7b);
}

#[test]
fn generated_frontend_reentrant_stack_pointer_deref_uses_stack_indirect() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-stack-pointer-deref",
        "
        module ReentrantStackPointerDerefDemo

        fixed($040006) volatile var out: u8

        export far procedure main()
            write($4006)
            return
        end

        reentrant procedure write(addr: u16)
            var p: near ^u8

            p = addr
            p^ = $5a
            out = p^
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040006), 0x5a);
}

#[test]
fn generated_frontend_reentrant_pointer_string_compare_runs() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-pointer-string-compare",
        "
        module ReentrantPointerStringCompareDemo

        fixed($040008) volatile var match_out: u8
        fixed($00400a) var left_buffer: [6]u8
        fixed($004010) var right_buffer: [6]u8

        export far procedure main()
            left_buffer[0] = 'g'
            left_buffer[1] = 'a'
            left_buffer[2] = 'm'
            left_buffer[3] = 'e'
            left_buffer[4] = 's'
            left_buffer[5] = 0
            right_buffer[0] = 'G'
            right_buffer[1] = 'A'
            right_buffer[2] = 'M'
            right_buffer[3] = 'E'
            right_buffer[4] = 'S'
            right_buffer[5] = 0
            match_out = text_matches($400a, $4010)
            return
        end

        function normalized(value: u8): u8
            if value >= 'a' and value <= 'z'
                return value - 32
            else
                return value
            end
        end

        reentrant function text_matches(left_text: near ^u8, right_text: near ^u8): u8
            var left: u8
            var right: u8
            while left_text^ != 0 and right_text^ != 0
                left = normalized(left_text^)
                right = normalized(right_text^)
                if left != right
                    return 0
                end
                left_text = left_text + 1
                right_text = right_text + 1
            end
            if left_text^ == 0 and right_text^ == 0
                return 1
            else
                return 0
            end
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040008), 1);
}

#[test]
fn generated_frontend_reentrant_call_args_preserve_stack_offsets_after_pushes() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-call-args-preserve-stack-offsets",
        "
        module ReentrantCallArgStackOffsetDemo

        fixed($400a) var entry_buffer: [16]u8
        fixed($4010) var name_buffer: [4]u8

        export far procedure main()
            name_buffer[0] = 'S'
            name_buffer[1] = 'Y'
            name_buffer[2] = 'S'
            name_buffer[3] = 0
            start()
            return
        end

        reentrant procedure start()
            var name: near ^u8
            name = $4010
            write_entry($400a, name, 65, 0)
            return
        end

        reentrant procedure write_entry(dta: near ^u8, name: near ^u8, size: u16, attr: u8)
            dta^ = attr
            dta = dta + 1
            dta^ = u8(size)
            dta = dta + 1
            dta^ = 0
            dta = dta + 1
            dta^ = name^
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x02400a), 0);
    assert_eq!(memory.byte(0x02400b), 65);
    assert_eq!(memory.byte(0x02400c), 0);
    assert_eq!(memory.byte(0x02400d), b'S');
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
fn generated_frontend_record_fields_run() {
    let binary = assemble_source_fixture(
        "frontend-record-fields",
        "
        module RecordFieldDemo
        type Device = record
            id: u16
            flags: u8
        end

        fixed($04000a) volatile var byte_out: u8
        fixed($04000c) volatile var word_out: u16
        var device: Device

        export far procedure main()
            device.id = $4567
            device.flags = $a5
            word_out = device.id
            byte_out = device.flags
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x04000a), 0xa5);
    assert_eq!(memory.byte(0x04000c), 0x67);
    assert_eq!(memory.byte(0x04000d), 0x45);
}

#[test]
fn generated_frontend_stack_record_fields_run() {
    let binary = assemble_source_fixture(
        "frontend-stack-record-fields",
        "
        module StackRecordFieldDemo
        type Device = record
            id: u16
            flags: u8
        end

        fixed($04000e) volatile var byte_out: u8
        fixed($040010) volatile var word_out: u16

        export far procedure main()
            work()
            return
        end

        reentrant procedure work()
            var device: Device
            device.id = $2345
            device.flags = $5a
            word_out = device.id
            byte_out = device.flags
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x04000e), 0x5a);
    assert_eq!(memory.byte(0x040010), 0x45);
    assert_eq!(memory.byte(0x040011), 0x23);
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
fn generated_frontend_reentrant_stack_function_call_runs() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-stack-function-call",
        "
        module ReentrantStackFunctionDemo
        fixed($040012) volatile var byte_out: u8
        fixed($040014) volatile var word_out: u16

        export far procedure main()
            word_out = add(2, $1234)
            if add(1, $1234) == $1235
                byte_out = 'Y'
            else
                byte_out = 'N'
            end
            return
        end

        reentrant function add(a: u16, b: u16): u16
            return b + a
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040012), b'Y');
    assert_eq!(memory.byte(0x040014), 0x36);
    assert_eq!(memory.byte(0x040015), 0x12);
}

#[test]
fn generated_frontend_reentrant_stack_temps_keep_parameter_offsets() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-stack-temps-keep-parameter-offsets",
        "
        module ReentrantStackTempDemo
        fixed($040016) volatile var byte_out: u8
        fixed($040018) volatile var word_out: u16

        export far procedure main()
            write_value('K', 5)
            return
        end

        reentrant procedure write_value(value: u8, add: u16)
            var tmp: u16
            tmp = bump(add)
            word_out = tmp
            byte_out = value
            return
        end

        reentrant function bump(value: u16): u16
            return value + 1 & 63
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040016), b'K');
    assert_eq!(memory.byte(0x040018), 6);
    assert_eq!(memory.byte(0x040019), 0);
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

    let (_cpu, memory) = run_generated_near_routine(&binary);

    assert_eq!(memory.byte(0x040006), 0x00);
    assert_eq!(memory.byte(0x040007), 0x2a);
}

#[test]
fn generated_frontend_shifts_and_bitwise_or_run() {
    let binary = assemble_source_fixture(
        "frontend-shifts-or",
        "
        module ShiftsDemo
        fixed($040020) volatile var out: [9]u8

        export far procedure main()
            var w: u16
            var b: u8
            w = $1234
            out[0] = u8(w << 4)
            out[1] = u8(w >> 4)
            w = w << 5
            out[2] = u8(w)
            out[3] = u8(w >> 8)
            w = $00ab
            w = w << 8 | $00cd
            out[4] = u8(w)
            out[5] = u8(w >> 8)
            w = $8001 >> 15
            out[6] = u8(w)
            b = $81
            out[7] = b << 1 | b >> 7
            out[8] = $f0 | $0f
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    let out = (0..9)
        .map(|i| memory.byte(0x040020 + i))
        .collect::<Vec<_>>();
    assert_eq!(
        out,
        vec![0x40, 0x23, 0x80, 0x46, 0xcd, 0xab, 0x01, 0x03, 0xff]
    );
}

#[test]
fn generated_frontend_reentrant_indexing_uses_stack_index() {
    let binary = assemble_source_fixture(
        "frontend-reentrant-indexing",
        "
        module ReentrantIndexDemo
        fixed($040030) volatile var out: u8
        var table: [8]u8

        export far procedure main()
            fill(3)
            return
        end

        reentrant procedure fill(start: u16)
            var i: u16
            i = start
            table[i + 1] = 'R'
            table[2] = 'C'
            out = table[i + 1]
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    assert_eq!(memory.byte(0x040030), b'R');
}

#[test]
fn generated_frontend_word_arrays_and_compare_operands_run() {
    let binary = assemble_source_fixture(
        "frontend-word-arrays-compare",
        "
        module WordArrayDemo
        fixed($040040) volatile var words: [4]u16
        fixed($040050) volatile var bytes: [8]u8
        fixed($040058) volatile var flags: u8

        export far procedure main()
            var i: u16
            i = 0
            while i != 4
                words[i] = $1100 + i
                i = i + 1
            end
            bytes[5] = 10
            bytes[6] = 11
            i = 3
            flags = 0
            if twice(5) == bytes[i + 2]
                flags = flags + 1
            end
            if twice(5) == bytes[i + 3]
                flags = flags + 2
            end
            return
        end

        function twice(value: u8): u8
            return value + value
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    let words = (0..8)
        .map(|i| memory.byte(0x040040 + i))
        .collect::<Vec<_>>();
    assert_eq!(words, vec![0x00, 0x11, 0x01, 0x11, 0x02, 0x11, 0x03, 0x11]);
    assert_eq!(memory.byte(0x040058), 1);
}

#[test]
fn generated_frontend_math_signed_and_routine_pointers_run() {
    let binary = assemble_source_fixture(
        "frontend-math",
        "
        module MathDemo
        fixed($040040) volatile var w: [12]u16
        fixed($040060) volatile var by: [8]u8
        fixed($040070) volatile var ss: [10]i16
        fixed($040090) volatile var f: [8]u8

        type Binop = reentrant function(a: u16, b: u16): u16
        type Thunk = procedure()

        var hits: u8

        export far procedure main()
            var x: u16
            var y: u16
            var c: u8
            var d: u8
            var sx: i16
            var sy: i16
            var sb: i8
            var op: Binop
            var t: Thunk
            x = 1234
            y = 56
            w[0] = x * y
            w[1] = x / y
            w[2] = x % y
            w[3] = x xor y
            c = 200
            d = 7
            by[0] = c * d
            by[1] = c / d
            by[2] = c % d
            sx = -1234
            sy = 56
            ss[0] = sx / sy
            ss[1] = sx % sy
            ss[2] = sx * sy
            ss[3] = sx >> 2
            sy = -7
            sx = 100
            ss[4] = sx / sy
            ss[5] = sx % sy
            sx = 5
            ss[6] = -sx
            ss[7] = 100 - sx * 3
            sb = -3
            ss[8] = i16(sb)
            sx = -5
            sy = 3
            f[0] = 0
            f[1] = 0
            f[2] = 0
            f[3] = 0
            if sx < sy
                f[0] = 1
            end
            if sx < 0
                f[1] = 1
            end
            if sx > -5
                f[2] = 1
            end
            if sy >= sx
                f[3] = 1
            end
            op = add
            w[4] = op(3, 4)
            op = mul
            w[5] = apply(op, 6, 7)
            w[6] = apply(add, 100, 23)
            t = bump
            hits = 0
            t()
            t()
            by[3] = hits
            return
        end

        reentrant function add(a: u16, b: u16): u16
            return a + b
        end

        reentrant function mul(a: u16, b: u16): u16
            return a * b
        end

        procedure bump()
            hits = hits + 1
            return
        end

        reentrant function apply(fn: Binop, a: u16, b: u16): u16
            return fn(a, b)
        end
        ",
    );

    let (_cpu, memory) = run_long_generated_routine(&binary);

    let word = |at: usize| u16::from_le_bytes([memory.byte(at), memory.byte(at + 1)]);
    let words: Vec<u16> = (0..7).map(|i| word(0x040040 + 2 * i)).collect();
    assert_eq!(
        words,
        vec![1234u16.wrapping_mul(56), 22, 2, 1234 ^ 56, 7, 42, 123]
    );
    let bytes: Vec<u8> = (0..4).map(|i| memory.byte(0x040060 + i)).collect();
    assert_eq!(bytes, vec![200u8.wrapping_mul(7), 28, 4, 2]);
    let signed: Vec<i16> = (0..9).map(|i| word(0x040070 + 2 * i) as i16).collect();
    assert_eq!(
        signed,
        vec![
            -1234 / 56,
            -1234 % 56,
            (-1234i16).wrapping_mul(56),
            -1234 >> 2,
            100 / -7,
            100 % -7,
            -5,
            85,
            -3,
        ]
    );
    let flags: Vec<u8> = (0..4).map(|i| memory.byte(0x040090 + i)).collect();
    assert_eq!(flags, vec![1, 1, 0, 1]);
}

#[test]
fn generated_frontend_data_tables_record_elements_and_for_loops_run() {
    let binary = assemble_source_fixture(
        "frontend-data",
        "
        module DataDemo
        fixed($040100) volatile var out: [16]u8
        fixed($040120) volatile var wout: [4]u16

        type Point = record(x: u8, y: u8)
        type Op = function(): u8

        const SQUARES: [6]u8 = [0, 1, 4, 9, 16, 25]
        const WIDE: [3]u16 = [$1234, -2, 2000]
        const GRID: [2][3]u8 = [[1, 2, 3], [4, 5, 6]]
        const OPS: [2]Op = [seven, nine]
        var points: [3]Point = [(1, 2), (10, 20), (100, 200)]

        export far procedure main()
            var i: u8
            var j: u16
            var k: u8
            var sum: u16
            var op: Op
            var p: near ^Point
            i = 4
            out[0] = SQUARES[i]
            j = 5
            out[1] = SQUARES[j]
            wout[0] = WIDE[1]
            i = 1
            k = 2
            out[2] = GRID[i][k]
            op = OPS[i]
            out[3] = op()
            i = 2
            out[4] = points[i].y
            points[i].y = 77
            out[5] = points[2].y
            p = addr(points) + size(Point)
            p^.x = p^.x + 5
            out[6] = points[1].x
            sum = 0
            for i = 1 to 10
                sum = sum + u16(i)
            end
            wout[1] = sum
            k = 0
            for i = 2 downto 0
                out[7 + u16(k)] = i
                k = k + 1
            end
            out[10] = 0
            if not (i == 2 or i == 4) and (k > 1)
                out[10] = 1
            end
            return
        end

        function seven(): u8
            return 7
        end

        function nine(): u8
            return 9
        end
        ",
    );

    let (_cpu, memory) = run_long_generated_routine(&binary);

    let out: Vec<u8> = (0..11).map(|i| memory.byte(0x040100 + i)).collect();
    assert_eq!(out, vec![16, 25, 6, 9, 200, 77, 15, 2, 1, 0, 1]);
    let word = |at: usize| u16::from_le_bytes([memory.byte(at), memory.byte(at + 1)]);
    assert_eq!(word(0x040120), 0xfffe);
    assert_eq!(word(0x040122), 55);
}

#[test]
fn generated_frontend_stack_routines_run() {
    let binary = assemble_source_fixture(
        "frontend-stack",
        "
        module StackDemo
        fixed($040200) volatile var out: [3]u16
        type Combine = stack function(a: u16, b: u8): u16

        export far procedure main()
            var combine: Combine
            out[0] = spread(1000, 5, 2000, 7)
            combine = weigh
            out[1] = combine(300, 4)
            out[2] = bump(21)
            return
        end
        stack function weigh(a: u16, b: u8): u16
            var kept: u16
            kept = a + u16(b)
            kept = kept + bump(b)
            return kept
        end

        stack function bump(b: u8): u16
            var twice: u16
            twice = u16(b) + u16(b)
            return twice
        end

        stack function spread(a: u16, b: u8, c: u16, d: u8): u16
            var left: u16
            var right: u16
            left = weigh(a, b)
            right = weigh(c, d)
            return left + right
        end
        ",
    );

    let (_cpu, memory) = run_long_generated_routine(&binary);

    let word = |at: usize| u16::from_le_bytes([memory.byte(at), memory.byte(at + 1)]);
    assert_eq!(
        (word(0x040200), word(0x040202), word(0x040204)),
        (1015 + 2021, 312, 42)
    );
}

#[test]
fn generated_frontend_register_arguments_and_carry_results_run() {
    let binary = assemble_source_fixture(
        "frontend-register-arguments",
        "
        module Registers
        fixed($040010) volatile var out: [8]u8
        fixed($040020) var total: u16
        const TABLE: [4]u8 = [10, 20, 30, 40]

        export far procedure main()
            var first: u8
            first = pick(2, 5)
            out[0] = first
            out[1] = pick(first - 32, pick(0, 1))
            total = 0
            add($1234, 2)
            add($0101, first)
            out[2] = u8(total)
            out[3] = u8(total >> 8)
            out[4] = is_even(4)
            out[5] = is_even(7)
            if is_even(10) and not below(9, 3)
                out[6] = 1
            end
            if below(3, 9)
                out[7] = 1
            end
            return
        end

        function pick(index: u8 in x, bias: u8 in a): u8
            return TABLE[index] + bias
        end

        procedure add(amount: u16 in xy, extra: u8)
            total = total + amount + u16(extra)
            return
        end

        function is_even(value: u8 in a): u8 in carry
            if value & 1
                return 0
            end
            return 1
        end

        function below(value: u8 in a, limit: u8 in y): u8 in carry
            if value < limit
                return 1
            end
            return 0
        end

        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    let got: Vec<u8> = (0..8).map(|i| memory.byte(0x040010 + i)).collect();
    assert_eq!(got, [35, 51, 0x5a, 0x13, 1, 0, 1, 1]);
}

#[test]
fn generated_frontend_field_calls_and_element_addresses_run() {
    let binary = assemble_source_fixture(
        "frontend-field-calls",
        "
        module FieldCalls
        fixed($040010) volatile var out: [6]u8
        fixed($040020) var argument: u16
        type Node = record(value: u16, next: near ^Node)
        fixed($004030) var nodes: [3]Node
        type Handler = stack procedure(arg: u16)
        type Command = record(name: near ^u8, run: Handler)

        const COMMANDS: parallel [3]Command = [
            (\"DIR\", run_dir),
            (\"TYPE\", run_type),
            (\"COPY\", run_copy),
        ]

        export far procedure main()
            var i: u8
            var p: near ^Node
            var names: near ^near ^u8
            var name: near ^u8
            argument = 7
            for i = 0 to 2
                COMMANDS[i].run(argument + u16(i))
            end
            # The name column of a parallel table, element 1.
            names = addr(COMMANDS[1].name)
            name = names^
            out[3] = name^
            nodes[0].value = 5
            nodes[0].next = addr(nodes[2])
            nodes[2].value = 9
            nodes[2].next = 0
            p = addr(nodes[0])
            out[4] = u8(p^.next^.value)
            return
        end

        stack procedure run_dir(arg: u16)
            out[0] = u8(arg)
            return
        end

        stack procedure run_type(arg: u16)
            out[1] = u8(arg)
            return
        end

        stack procedure run_copy(arg: u16)
            out[2] = u8(arg)
            return
        end
        ",
    );

    let (_cpu, memory) = run_generated_routine(&binary);

    let got: Vec<u8> = (0..5).map(|i| memory.byte(0x040010 + i)).collect();
    assert_eq!(got, [7, 8, 9, b'T', 9]);
}
