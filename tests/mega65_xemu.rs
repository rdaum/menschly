//! End-to-end MEGA65 tests: Mensch source -> 45GS02 assembly -> 128 KiB ROM
//! image -> headless Xemu boot. Each program exits through Xemu's testing
//! register with `main`'s result as the process status, and the test then
//! inspects the RAM dump.
//!
//! Skipped (with a note on stderr) when no Xemu binary is found; see
//! `mega65::find_xemu`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mensch_systems_tests::driver::{self, Target};
use mensch_systems_tests::mega65::{self, XemuRun};

static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "mensch-mega65-{name}-{}-{nanos}-{id}",
        std::process::id()
    ))
}

fn compile_rom(name: &str, source: &str) -> (PathBuf, Vec<u8>) {
    let compiled =
        driver::compile_source(source, Target::Mega65, driver::DEFAULT_DIRECT_PAGE_BUDGET)
            .unwrap_or_else(|error| panic!("{name}: compile failed: {error}"));
    let dir = temp_dir(name);
    let rom = mega65::build_rom(&dir, &[(name, &compiled.asm)])
        .unwrap_or_else(|error| panic!("{name}: ROM build failed: {error}\n{}", compiled.asm));
    (dir, rom)
}

/// Compiles and boots `source`; returns `None` when Xemu is unavailable.
fn run_source(name: &str, source: &str) -> Option<XemuRun> {
    // Always check that the program compiles and links, even without Xemu.
    let (dir, rom) = compile_rom(name, source);
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping {name}: Xemu not found (set MENSCH_XMEGA65)");
        return None;
    };
    Some(
        mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("{name}: {error}")),
    )
}

#[test]
fn returns_exit_status_from_main() {
    let Some(run) = run_source(
        "exit-status",
        "
        module ExitStatus
        export function main(): u8
            return 42
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 42);
}

#[test]
fn control_flow_and_function_calls() {
    let Some(run) = run_source(
        "control-flow",
        "
        module FrontendDemo
        fixed($0800) volatile var out: u8

        export function main(): u8
            direct var p: near ^u8
            var x: u8

            p = $0801
            x = one()
            if x == 1
                x = x + 1
            end
            while x != 4
                x = x + 1
            end
            p^ = x
            out = $5a
            return x
        end

        function one(): u8
            return 1
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 4);
    assert_eq!(run.memory[0x0800], 0x5a);
    assert_eq!(run.memory[0x0801], 4);
}

#[test]
fn word_arithmetic_and_relational_conditions() {
    let Some(run) = run_source(
        "relational",
        "
        module RelationalConditionsDemo
        fixed($0806) volatile var byte_out: u8
        fixed($0808) volatile var word_out: u16
        fixed($080a) volatile var carry_out: u16

        export function main(): u8
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
            if y < 256
                word_out = word_out + 100
            end
            carry_out = $00ff
            carry_out = carry_out + 1
            carry_out = carry_out + $1234
            carry_out = carry_out - 2
            return 0
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    assert_eq!(run.memory[0x0806], 3);
    assert_eq!(run.memory[0x0808], 12);
    assert_eq!(run.memory[0x0809], 0);
    // $00ff + 1 + $1234 - 2 = $1332
    assert_eq!(run.memory[0x080a], 0x32);
    assert_eq!(run.memory[0x080b], 0x13);
}

#[test]
fn parameters_records_and_indexing() {
    let Some(run) = run_source(
        "params-records",
        "
        module ParamsRecords
        type Device = record
            id: u16
            flags: u8
        end

        fixed($0810) volatile var byte_out: u8
        fixed($0812) volatile var word_out: u16
        fixed($0814) volatile var id_out: u16
        fixed($0816) volatile var flag_out: u8
        fixed($0817) volatile var index_out: u8
        var device: Device
        var source: [4]u8
        var target: [4]u8

        export function main(): u8
            var src: u16
            var dst: u16
            store_pair('Q', $1234)
            device.id = $4567
            device.flags = $a5
            id_out = device.id
            flag_out = device.flags
            src = 2
            dst = 1
            source[src] = 'Z'
            target[dst] = source[src]
            index_out = target[dst]
            return add(2, 3)
        end

        procedure store_pair(first: u8, second: u16)
            byte_out = first
            word_out = second
            return
        end

        function add(a: u8, b: u8): u8
            return a + b
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 5);
    assert_eq!(run.memory[0x0810], b'Q');
    assert_eq!(&run.memory[0x0812..0x0814], &[0x34, 0x12]);
    assert_eq!(&run.memory[0x0814..0x0816], &[0x67, 0x45]);
    assert_eq!(run.memory[0x0816], 0xa5);
    assert_eq!(run.memory[0x0817], b'Z');
}

#[test]
fn word_function_results_and_pointer_walks() {
    let Some(run) = run_source(
        "word-results",
        "
        module WordResults
        fixed($0820) volatile var word_out: u16
        fixed($0822) var text: [4]u8
        fixed($0826) volatile var length_out: u8

        export function main(): u8
            text[0] = 'a'
            text[1] = 'b'
            text[2] = 'c'
            text[3] = 0
            word_out = double($1234)
            length_out = length($0822)
            return length_out
        end

        function double(value: u16): u16
            return value + value
        end

        function length(s: near ^u8): u8
            var n: u8
            n = 0
            while s^ != 0
                n = n + 1
                s = s + 1
            end
            return n
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 3);
    assert_eq!(&run.memory[0x0820..0x0822], &[0x68, 0x24]);
    assert_eq!(run.memory[0x0826], 3);
}

#[test]
fn reentrant_stack_parameters_locals_and_results() {
    let Some(run) = run_source(
        "reentrant-basics",
        "
        module ReentrantBasics
        fixed($0830) volatile var byte_out: u8
        fixed($0832) volatile var word_out: u16
        fixed($0834) volatile var sum_out: u16
        fixed($0836) volatile var local_out: u8
        fixed($0837) volatile var cmp_out: u8

        export function main(): u8
            store_pair('Z', $4567)
            sum_out = add(2, $1234)
            if add(1, $1234) == $1235
                cmp_out = 'Y'
            else
                cmp_out = 'N'
            end
            work('A')
            return bump(5)
        end

        reentrant procedure store_pair(a: u8, b: u16)
            byte_out = a
            word_out = b
            return
        end

        reentrant function add(a: u16, b: u16): u16
            return b + a
        end

        reentrant procedure work(a: u8)
            var tmp: u8
            tmp = a + 1
            local_out = tmp + 1
            return
        end

        reentrant function bump(value: u16): u8
            var tmp: u16
            tmp = value + 1 & 63
            return u8(tmp)
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 6);
    assert_eq!(run.memory[0x0830], b'Z');
    assert_eq!(&run.memory[0x0832..0x0834], &[0x67, 0x45]);
    assert_eq!(&run.memory[0x0834..0x0836], &[0x36, 0x12]);
    assert_eq!(run.memory[0x0836], b'C');
    assert_eq!(run.memory[0x0837], b'Y');
}

#[test]
fn reentrant_pointer_parameters_and_nested_calls() {
    let Some(run) = run_source(
        "reentrant-pointers",
        "
        module ReentrantPointers
        fixed($0840) volatile var match_out: u8
        fixed($0844) var left_buffer: [6]u8
        fixed($084a) var right_buffer: [6]u8
        fixed($0850) var entry_buffer: [4]u8

        export function main(): u8
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
            match_out = text_matches($0844, $084a)
            start()
            return match_out
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

        reentrant procedure start()
            var name: near ^u8
            name = $084a
            write_entry($0850, name, 65, 7)
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
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 1);
    assert_eq!(run.memory[0x0840], 1);
    assert_eq!(&run.memory[0x0850..0x0854], &[7, 65, 0, b'G']);
}

#[test]
fn reentrant_recursion_keeps_separate_frames() {
    let Some(run) = run_source(
        "reentrant-recursion",
        "
        module Recursion
        fixed($0860) volatile var out: u16

        export function main(): u8
            out = sum_to(20)
            return u8(fib(10))
        end

        reentrant function sum_to(n: u16): u16
            var rest: u16
            if n == 0
                return 0
            end
            rest = sum_to(n - 1)
            return rest + n
        end

        reentrant function fib(n: u16): u16
            var a: u16
            var b: u16
            if n < 2
                return n
            end
            a = fib(n - 1)
            b = fib(n - 2)
            return a + b
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 55);
    // 1 + 2 + ... + 20 = 210
    assert_eq!(&run.memory[0x0860..0x0862], &[210, 0]);
}

#[test]
fn reports_unsupported_cop_traps() {
    let error = driver::compile_source(
        "
        module Traps
        interrupt(cop) function dispatch(): u16
            return cop_a()
        end
        ",
        Target::Mega65,
        driver::DEFAULT_DIRECT_PAGE_BUDGET,
    )
    .expect_err("COP traps have no 45GS02 lowering");
    assert!(error.contains("45GS02 backend does not support"), "{error}");
}

#[test]
fn shifts_and_bitwise_or() {
    let Some(run) = run_source(
        "shifts",
        "
        module Shifts
        fixed($0870) volatile var out: [12]u8

        export function main(): u8
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
            return out[7]
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 0x03);
    let out = &run.memory[0x0870..0x0879];
    assert_eq!(out, &[0x40, 0x23, 0x80, 0x46, 0xcd, 0xab, 0x01, 0x03, 0xff]);
}

#[test]
fn indexing_beyond_255_bytes() {
    let Some(run) = run_source(
        "wide-index",
        "
        module WideIndex
        fixed($1000) var sector: [512]u8

        export function main(): u8
            var i: u16
            i = 0
            while i != 512
                sector[i] = u8(i >> 1)
                i = i + 1
            end
            sector[511] = $5a
            return sector[300]
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 150);
    assert_eq!(run.memory[0x1000 + 2], 1);
    assert_eq!(run.memory[0x1000 + 300], 150);
    assert_eq!(run.memory[0x1000 + 510], 255);
    assert_eq!(run.memory[0x1000 + 511], 0x5a);
}

#[test]
fn traps_dispatch_through_the_vector_table() {
    let Some(run) = run_source(
        "traps",
        "
        module Traps
        vector trap_01 = gemdos
        vector trap_0d = bios
        fixed($0880) volatile var out: [6]u8

        export function main(): u8
            var r: u16
            r = trap($0d, 7, $0100, $0023)
            out[0] = u8(r)
            out[1] = u8(r >> 8)
            r = trap($01, 2, 40, 2)
            out[2] = u8(r)
            out[3] = u8(trap($0d, 9, 0, 0))
            return out[0]
        end

        reentrant function bios(call: u16, arg0: u16, arg1: u16): u16
            if call == 7
                return arg0 | arg1
            end
            return $ffff
        end

        reentrant function gemdos(call: u16, arg0: u16, arg1: u16): u16
            return arg0 + arg1 + call
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 0x23);
    assert_eq!(&run.memory[0x0880..0x0884], &[0x23, 0x01, 44, 0xff]);
}

#[test]
fn large_kernel_runs_from_ram_above_the_rom_window() {
    // ~41 KiB of code: the last routines land above $A000 (BASIC ROM space
    // at reset) and must have been copied there by the loader.
    let mut source = String::from(
        "
        module LargeKernel
        fixed($0890) volatile var out: u16
        fixed($e100) volatile var high_ram: u8

        export function main(): u8
            out = 0
            high_ram = $a5
            r0()
            r49()
            return high_ram
        end
        ",
    );
    for n in 0..50 {
        source.push_str(&format!("\nprocedure r{n}()\n"));
        for _ in 0..48 {
            source.push_str("    out = out + 1\n");
        }
        source.push_str("    return\nend\n");
    }
    let (dir, rom) = compile_rom("large-kernel", &source);
    assert!(
        rom[0xa000..0xd000].iter().any(|byte| *byte != 0),
        "test program should extend into $A000-$CFFF"
    );
    let Some(xemu) = mega65::find_xemu() else {
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    assert_eq!(run.exit_code, 0xa5);
    assert_eq!(&run.memory[0x0890..0x0892], &[96, 0]);
}

#[test]
fn interrupts_preserve_runtime_registers_and_scratch() {
    let Some(run) = run_source(
        "irq-stress",
        "
        module IrqStress
        vector irq = tick
        fixed($dc04) volatile var cia1_ta_lo: u8
        fixed($dc05) volatile var cia1_ta_hi: u8
        fixed($dc0d) volatile var cia1_icr: u8
        fixed($dc0e) volatile var cia1_cra: u8
        fixed($d01a) volatile var vic_irq_mask: u8
        fixed($0900) volatile var ticks: u16
        fixed($0902) volatile var result: u16
        fixed($0904) volatile var isr_scratch: u16

        export function main(): u8
            var i: u16
            var sum: u16
            vic_irq_mask = 0
            cia1_icr = $7f
            ticks = 0
            cia1_ta_lo = $f4
            cia1_ta_hi = $01
            cia1_icr = $81
            cia1_cra = $11
            asm \"cli\"
            i = 0
            sum = 0
            while i != 4000
                sum = sum + (i << 3 | i >> 5)
                i = i + 1
            end
            asm \"sei\"
            result = sum
            return 0
        end

        interrupt procedure tick()
            var ack: u8
            ack = cia1_icr
            ticks = ticks + 1
            isr_scratch = ticks << 4 | ticks >> 3
            return
        end
        ",
    ) else {
        return;
    };
    let expected = (0u16..4000).fold(0u16, |sum, i| sum.wrapping_add((i << 3) | (i >> 5)));
    let ticks = u16::from_le_bytes([run.memory[0x0900], run.memory[0x0901]]);
    assert_eq!(run.exit_code, 0);
    assert!(ticks > 20, "expected many timer interrupts, got {ticks}");
    assert_eq!(
        u16::from_le_bytes([run.memory[0x0902], run.memory[0x0903]]),
        expected
    );
}

const HELLORD_MEGA65_MODULES: [&str; 8] = [
    "hellord-os",
    "hellord-dos",
    "hellord-console",
    "hellord-files",
    "mega65-machine",
    "hellord-bios",
    "hellord-gemdos",
    "hellord-traps-mega65",
];

fn hellord_rom(name: &str) -> (PathBuf, Vec<u8>) {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let sources = HELLORD_MEGA65_MODULES
        .iter()
        .enumerate()
        .map(|(index, module)| {
            let source = std::fs::read_to_string(examples.join(format!("{module}.mensch")))
                .expect("Hellord source should be readable");
            (format!("{index}-{module}"), source)
        })
        .collect::<Vec<_>>();
    let modules = mega65::compile_modules(&sources).expect("Hellord should compile for the MEGA65");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir(name);
    let rom = mega65::build_rom(&dir, &refs).expect("Hellord ROM should link");
    (dir, rom)
}

#[test]
fn hellord_boots_to_the_dos_prompt() {
    let (dir, rom) = hellord_rom("hellord-boot");
    let Some(xemu) = mega65::find_xemu() else {
        return;
    };
    let run = mega65::run_rom_with(
        &xemu,
        &dir,
        &rom,
        &mega65::XemuOptions {
            timeout: Duration::from_secs(3),
            stop_at_timeout: true,
            ..mega65::XemuOptions::default()
        },
    )
    .unwrap();
    assert!(run.timed_out);
    assert!(
        run.screen
            .starts_with("Hellord DOS 0.1\nType H for help.\nA>"),
        "screen:\n{}",
        run.screen
    );
}

#[test]
fn hellord_runs_typed_commands_and_reboots() {
    let (dir, rom) = hellord_rom("hellord-commands");
    let Some(xemu) = mega65::find_xemu() else {
        return;
    };
    if !mega65::xemu_supports_type_in(&xemu) {
        eprintln!("skipping: this Xemu build has no -typein option");
        return;
    }
    let run = mega65::run_rom_with(
        &xemu,
        &dir,
        &rom,
        &mega65::XemuOptions {
            type_in: Some("VER\\rCLS\\rHELP\\rREBOOT\\r".to_owned()),
            ..mega65::XemuOptions::default()
        },
    )
    .unwrap();
    assert!(!run.timed_out, "REBOOT should end the run");
    assert!(
        run.screen.starts_with("Commands:"),
        "screen:\n{}",
        run.screen
    );
    assert!(
        run.screen.contains("FILL addr len byte"),
        "screen:\n{}",
        run.screen
    );
    assert!(run.screen.contains("A> REBOOT"), "screen:\n{}", run.screen);
}

#[test]
fn sd_card_block_device_reads_and_writes_the_fat_partition() {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let machine = std::fs::read_to_string(examples.join("mega65-machine.mensch")).unwrap();
    let caller = "
        module SdTest
        external reentrant function blockdev_status(): u16
        external reentrant function blockdev_sector_count(): u16
        external reentrant procedure blockdev_set_lba(lba: u16)
        external reentrant function blockdev_read(buffer: near ^u8): u16
        external reentrant function blockdev_write(buffer: near ^u8): u16
        fixed($0900) volatile var status: u16
        fixed($0902) volatile var sectors: u16
        fixed($0904) volatile var read_error: u16
        fixed($0906) volatile var write_error: u16
        fixed($0908) volatile var reread_error: u16
        fixed($1000) var boot: [512]u8
        fixed($1200) var pattern: [512]u8
        fixed($1400) var check: [512]u8

        export function main(): u8
            var i: u16
            status = blockdev_status()
            sectors = blockdev_sector_count()
            blockdev_set_lba(0)
            read_error = blockdev_read($1000)
            i = 0
            while i != 512
                pattern[i] = u8(i) + u8(i >> 8)
                i = i + 1
            end
            blockdev_set_lba(4000)
            write_error = blockdev_write($1200)
            reread_error = blockdev_read($1400)
            return 0
        end
    ";
    let modules = mega65::compile_modules(&[
        ("0-sd-test".to_owned(), caller.to_owned()),
        ("1-mega65-machine".to_owned(), machine),
    ])
    .expect("SD test should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir("sd-card");
    let rom = mega65::build_rom(&dir, &refs).expect("SD test ROM should link");
    let Some(xemu) = mega65::find_xemu() else {
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    let word = |addr: usize| u16::from_le_bytes([run.memory[addr], run.memory[addr + 1]]);
    assert_eq!(run.exit_code, 0);
    assert_eq!(word(0x0900), 3, "status: present and ready");
    assert_ne!(word(0x0902), 0, "partition size");
    assert_eq!(word(0x0904), 0, "read error");
    let boot = &run.memory[0x1000..0x1200];
    assert_eq!(&boot[510..512], &[0x55, 0xaa], "boot sector signature");
    assert_eq!(&boot[0x52..0x5a], b"FAT32   ", "FAT32 boot sector");
    assert_eq!(word(0x0906), 0, "write error");
    assert_eq!(word(0x0908), 0, "reread error");
    assert_eq!(&run.memory[0x1400..0x1600], &run.memory[0x1200..0x1400]);
}

#[test]
fn hellord_blkinfo_reads_the_sd_card_partition() {
    let (dir, rom) = hellord_rom("hellord-blkinfo");
    let Some(xemu) = mega65::find_xemu() else {
        return;
    };
    if !mega65::xemu_supports_type_in(&xemu) {
        eprintln!("skipping: this Xemu build has no -typein option");
        return;
    }
    let run = mega65::run_rom_with(
        &xemu,
        &dir,
        &rom,
        &mega65::XemuOptions {
            type_in: Some("BLKINFO\\rREBOOT\\r".to_owned()),
            ..mega65::XemuOptions::default()
        },
    )
    .unwrap();
    assert!(!run.timed_out, "REBOOT should end the run");
    assert!(
        run.screen
            .contains("A> BLKINFO\nDrive A block:\nStatus $03\n"),
        "screen:\n{}",
        run.screen
    );
    // Xemu's virtual card: a FAT32 partition whose boot sector starts with a
    // jump and the OEM name "MEGA65r1".
    assert!(
        run.screen.contains("Sector0: EB 58 90 4D 45 47 41 36 35"),
        "screen:\n{}",
        run.screen
    );
}
