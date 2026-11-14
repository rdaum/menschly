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
        // `+ 3` keeps the full add (an increment would be much smaller).
        for _ in 0..48 {
            source.push_str("    out = out + 3\n");
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
    assert_eq!(&run.memory[0x0890..0x0892], &[0x20, 0x01]);
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

/// Hellord's MEGA65 kernel modules and their window banks (as in
/// `scripts/boot-hellord-mega65.sh`).
const HELLORD_MEGA65_MODULES: [(&str, Option<u8>); 10] = [
    ("hellord-os", None),
    ("hellord-dos", Some(1)),
    ("hellord-fat", Some(2)),
    ("hellord-exec", Some(2)),
    ("hellord-console", None),
    ("hellord-files", None),
    ("mega65-machine", None),
    ("hellord-bios", None),
    ("hellord-gemdos", None),
    ("hellord-traps-mega65", None),
];

fn hellord_rom(name: &str) -> (PathBuf, Vec<u8>) {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let sources = HELLORD_MEGA65_MODULES
        .iter()
        .enumerate()
        .map(|(index, (module, window))| mega65::KernelModule {
            name: format!("{index}-{module}"),
            source: std::fs::read_to_string(examples.join(format!("{module}.mensch")))
                .expect("Hellord source should be readable"),
            window: *window,
        })
        .collect::<Vec<_>>();
    let modules =
        mega65::compile_kernel_modules(&sources).expect("Hellord should compile for the MEGA65");
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

fn hellord_library_rom(name: &str, caller: &str) -> (PathBuf, Vec<u8>) {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut sources = vec![mega65::KernelModule {
        name: "0-caller".to_owned(),
        source: caller.to_owned(),
        window: None,
    }];
    for (index, (module, window)) in HELLORD_MEGA65_MODULES.iter().enumerate() {
        if *module == "hellord-os" {
            continue;
        }
        sources.push(mega65::KernelModule {
            name: format!("{}-{module}", index + 1),
            source: std::fs::read_to_string(examples.join(format!("{module}.mensch")))
                .expect("Hellord source should be readable"),
            window: *window,
        });
    }
    let modules = mega65::compile_kernel_modules(&sources).expect("program should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir(name);
    let rom = mega65::build_rom(&dir, &refs).expect("program should link");
    (dir, rom)
}

fn big_file() -> Vec<u8> {
    (0..10_000u32).map(|i| (i * 7 + i / 251) as u8).collect()
}

#[test]
fn gemdos_reads_fat32_files_from_the_sd_card() {
    let (dir, rom) = hellord_library_rom(
        "fat32-gemdos",
        "
        module Fat32Caller
        vector irq = serial_irq
        external interrupt procedure serial_irq()
        external procedure serial_init()
        external reentrant procedure gemdos_init()
        external reentrant function gemdos_dsetdrv(drive: u16): u16
        external reentrant function gemdos_fsfirst(pattern: near ^u8, dta: near ^u8): u16
        external reentrant function gemdos_fsnext(dta: near ^u8): u16
        external reentrant function gemdos_fopen(path: near ^u8, mode: u16): u16
        external reentrant function gemdos_fread(handle: u16, count: u16, buffer: near ^u8): u16
        external reentrant function gemdos_fclose(handle: u16): u16
        fixed($0900) volatile var entries: u16
        fixed($0902) volatile var handle: u16
        fixed($0904) volatile var total: u16
        fixed($0906) volatile var sum: u16
        fixed($0908) volatile var small_handle: u16
        fixed($090a) volatile var small_count: u16
        fixed($0a00) var dta: [32]u8
        fixed($0b00) var chunk: [300]u8
        fixed($0c40) var small: [64]u8

        export function main(): u8
            var got: u16
            var i: u16
            serial_init()
            gemdos_init()
            got = gemdos_dsetdrv(18)
            entries = 0
            got = gemdos_fsfirst(\"*.*\", $0a00)
            while got == 0
                entries = entries + 1
                got = gemdos_fsnext($0a00)
            end
            small_handle = gemdos_fopen(\"hello.txt\", 0)
            small_count = gemdos_fread(small_handle, 64, $0c40)
            got = gemdos_fclose(small_handle)
            handle = gemdos_fopen(\"BIG.DAT\", 0)
            total = 0
            sum = 0
            got = 1
            while got != 0 and got < $fff0
                got = gemdos_fread(handle, 300, $0b00)
                if got < $fff0
                    i = 0
                    while i != got
                        sum = sum + u16(chunk[i])
                        i = i + 1
                    end
                    total = total + got
                end
            end
            got = gemdos_fclose(handle)
            return 0
        end
        ",
    );
    let Some(xemu) = mega65::find_xemu() else {
        return;
    };
    let mut sd_files = vec![
        (
            "HELLO.TXT".to_owned(),
            b"Hello from the SD card!\n".to_vec(),
        ),
        ("BIG.DAT".to_owned(), big_file()),
    ];
    // Enough entries that the root directory spans several sectors.
    for n in 0..20 {
        sd_files.push((format!("F{n:02}.TXT"), format!("file {n}\n").into_bytes()));
    }
    let run = mega65::run_rom_with(
        &xemu,
        &dir,
        &rom,
        &mega65::XemuOptions {
            sd_files,
            ..mega65::XemuOptions::default()
        },
    )
    .unwrap();
    let word = |addr: usize| u16::from_le_bytes([run.memory[addr], run.memory[addr + 1]]);
    assert_eq!(run.exit_code, 0, "log: {}", run.log);
    assert_eq!(word(0x0900), 22, "root directory entries");
    assert_eq!(word(0x090a), 24, "HELLO.TXT bytes read");
    assert_eq!(&run.memory[0x0c40..0x0c58], b"Hello from the SD card!\n");
    assert_eq!(word(0x0904), 10_000, "BIG.DAT bytes read");
    let expected = big_file()
        .iter()
        .fold(0u16, |sum, byte| sum.wrapping_add(u16::from(*byte)));
    assert_eq!(word(0x0906), expected, "BIG.DAT checksum");
}

#[test]
fn hellord_lists_and_types_files_on_the_sd_card() {
    let (dir, rom) = hellord_rom("hellord-fat32");
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
            type_in: Some("S:\\rDIR\\rTYPE hello.txt\\rTYPE NOPE.TXT\\rREBOOT\\r".to_owned()),
            sd_files: vec![
                (
                    "HELLO.TXT".to_owned(),
                    b"Hello from the SD card!\r\nSecond line.\n".to_vec(),
                ),
                ("BIG.DAT".to_owned(), big_file()),
            ],
            ..mega65::XemuOptions::default()
        },
    )
    .unwrap();
    assert!(!run.timed_out, "REBOOT should end the run");
    let expected = "S> DIR\nDirectory of S:/\nHELLO.TXT    $0026\nBIG.DAT      $2710\n02 file(s)  00 dir(s)\nS> TYPE hello.txt\nHello from the SD card!\nSecond line.\nS> TYPE NOPE.TXT\nFile not found\n";
    assert!(run.screen.contains(expected), "screen:\n{}", run.screen);
}

/// A loadable program that exercises every relocation kind the loader
/// handles: calls and jumps into its own text (WORD), string addresses
/// (LOW/HIGH), a BSS counter, and a base-page parameter (zero page).
const HELLO_PROGRAM: &str = "
module Hello
const GEMDOS = $01
const CCONOUT = $0002

var greetings: u16

procedure say(text: near ^u8)
    var ignored: u16
    while text^ != 0
        ignored = trap(GEMDOS, CCONOUT, u16(text^), 0)
        text = text + 1
    end
    return
end

export function main(): u8
    greetings = greetings + 1
    say(\"Hello from a program\\r\\n\")
    if greetings == 1
        say(\"BSS was cleared\\r\\n\")
    end
    return $2a
end
";

fn build_hello_program() -> Vec<u8> {
    let modules =
        mega65::compile_program_modules(&[("hello".to_owned(), HELLO_PROGRAM.to_owned())])
            .expect("program should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let program =
        mega65::build_program(&temp_dir("hello-program"), &refs).expect("program should link");
    assert_eq!(&program[..6], b"\x01\x00o65\x00", "o65 magic");
    assert!(
        program[0x15] != 0 || program[0x16] != 0,
        "the program should have a base-page segment"
    );
    program
}

#[test]
fn hellord_loads_and_runs_relocated_programs_from_the_sd_card() {
    let program = build_hello_program();
    let (dir, rom) = hellord_rom("hellord-exec");
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
            type_in: Some(
                "S:\rRUN HELLO\rhello.prg\rRUN NOTPROG.PRG\rMISSING\rREBOOT\r".to_owned(),
            ),
            sd_files: vec![
                ("HELLO.PRG".to_owned(), program),
                ("NOTPROG.PRG".to_owned(), b"just some text\r\n".to_vec()),
            ],
            ..mega65::XemuOptions::default()
        },
    )
    .unwrap();
    assert!(!run.timed_out, "REBOOT should end the run:\n{}", run.screen);
    let expected = "S> RUN HELLO\nHello from a program\nBSS was cleared\nExit status $2A\n\
                    S> hello.prg\nHello from a program\nBSS was cleared\nExit status $2A\n\
                    S> RUN NOTPROG.PRG\nNot a program\n\
                    S> MISSING\nProgram not found\n";
    assert!(run.screen.contains(expected), "screen:\n{}", run.screen);
}

#[test]
fn word_arrays_index_by_element_and_compares_keep_their_left_side() {
    let Some(run) = run_source(
        "index-scaling",
        "
        module IndexScaling
        fixed($08a0) volatile var words: [4]u16
        fixed($08c0) volatile var bytes: [8]u8
        fixed($08d0) volatile var flags: u8

        function twice(value: u8): u8
            return value + value
        end

        export function main(): u8
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
            # The right-hand side computes an index, which must not clobber
            # the left-hand call result.
            if twice(5) == bytes[i + 2]
                flags = flags + 1
            end
            if twice(5) == bytes[i + 3]
                flags = flags + 2
            end
            return flags
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 1);
    assert_eq!(
        &run.memory[0x08a0..0x08a8],
        &[0x00, 0x11, 0x01, 0x11, 0x02, 0x11, 0x03, 0x11]
    );
}

const LANGUAGE_FEATURES: &str = "
module LanguageFeatures
fixed($0a00) volatile var w: [16]u16
fixed($0a20) volatile var by: [16]u8
fixed($0a30) volatile var qs: [12]u32
fixed($0a60) volatile var f: [16]u8
fixed($0a70) volatile var ss: [10]i16

type Binop = reentrant function(a: u16, b: u16): u16
type Thunk = procedure()

var hits: u8
var op: Binop

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

reentrant function wide(a: u32, b: u32): u32
    return a * b + 1
end

function big(): u32
    return $12345678
end

reentrant function flag(value: u8): u8
    return value
end

export function main(): u8
    var x: u16
    var y: u16
    var c: u8
    var d: u8
    var sx: i16
    var sy: i16
    var sb: i8
    var l: u32
    var m: u32
    var sl: i32
    var t: Thunk
    var fp: far ^u8

    x = 1234
    y = 56
    w[0] = x * y
    w[1] = x / y
    w[2] = x % y
    w[3] = x * 8
    w[4] = x / 16
    w[5] = x % 16
    w[6] = x xor y
    y = 0
    w[7] = x / y
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
    sx = -100
    ss[6] = sx / sy
    ss[7] = sx % sy
    sx = 5
    ss[8] = -sx
    ss[9] = 100 - sx * 3

    sx = -5
    sy = 3
    if sx < sy
        f[0] = 1
    end
    if sy > sx
        f[1] = 1
    end
    if sx < 0
        f[2] = 1
    end
    if sx >= -5
        f[3] = 1
    end
    if sx > -5
        f[4] = 1
    else
        f[4] = 0
    end
    x = $fffb
    if x < 3
        f[5] = 1
    else
        f[5] = 0
    end

    sb = -3
    sx = i16(sb)
    w[8] = u16(sx)
    sl = i32(sx)
    qs[0] = u32(sl)
    c = $f0
    w[9] = u16(c)

    l = 100000
    m = 300
    qs[1] = l * m
    qs[2] = l / m
    qs[3] = l % m
    qs[4] = l + m - 1
    qs[5] = big()
    qs[6] = wide(l, 3)
    sl = -100000
    qs[7] = u32(sl / 7)
    if l > m
        f[6] = 1
    end
    if sl < 0
        f[7] = 1
    end
    l = $12345678
    qs[8] = l >> 4
    qs[9] = l << 8
    sl = -256
    qs[10] = u32(sl >> 4)
    qs[11] = u32(sl % 100)

    op = add
    w[10] = op(3, 4)
    op = mul
    w[11] = apply(op, 6, 7)
    w[12] = apply(add, 100, 23)
    t = bump
    hits = 0
    t()
    t()
    by[3] = hits
    f[8] = flag(u8(apply(add, 1, 1)))

    fp = $00000a20
    by[4] = fp^
    return 0
end
";

#[test]
fn multiply_divide_signed_32_bit_and_routine_pointers() {
    let Some(run) = run_source("language-features", LANGUAGE_FEATURES) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    let word =
        |i: usize| u16::from_le_bytes([run.memory[0x0a00 + 2 * i], run.memory[0x0a01 + 2 * i]]);
    let signed =
        |i: usize| i16::from_le_bytes([run.memory[0x0a70 + 2 * i], run.memory[0x0a71 + 2 * i]]);
    let quad = |i: usize| {
        let at = 0x0a30 + 4 * i;
        u32::from_le_bytes(run.memory[at..at + 4].try_into().unwrap())
    };
    let words: Vec<u16> = (0..13).map(word).collect();
    assert_eq!(
        words,
        vec![
            1234u16.wrapping_mul(56),
            1234 / 56,
            1234 % 56,
            1234 * 8,
            1234 / 16,
            1234 % 16,
            1234 ^ 56,
            0xffff,
            0xfffd,
            0x00f0,
            7,
            42,
            123,
        ]
    );
    assert_eq!(
        &run.memory[0x0a20..0x0a25],
        &[
            200u8.wrapping_mul(7),
            200 / 7,
            200 % 7,
            2,
            200u8.wrapping_mul(7)
        ]
    );
    let signeds: Vec<i16> = (0..10).map(signed).collect();
    assert_eq!(
        signeds,
        vec![
            -1234 / 56,
            -1234 % 56,
            (-1234i16).wrapping_mul(56),
            -1234 >> 2,
            100 / -7,
            100 % -7,
            -100 / -7,
            -100 % -7,
            -5,
            85,
        ]
    );
    let quads: Vec<u32> = (0..12).map(quad).collect();
    assert_eq!(
        quads,
        vec![
            -3i32 as u32,
            30_000_000,
            333,
            100,
            100_299,
            0x1234_5678,
            300_001,
            (-100_000i32 / 7) as u32,
            0x0123_4567,
            0x3456_7800,
            -16i32 as u32,
            (-256i32 % 100) as u32,
        ]
    );
    assert_eq!(&run.memory[0x0a60..0x0a69], &[1, 1, 1, 1, 0, 0, 1, 1, 2]);
}

#[test]
fn interrupts_can_multiply_and_divide_while_main_code_does() {
    let Some(run) = run_source(
        "irq-math",
        "
        module IrqMath
        vector irq = tick
        fixed($dc04) volatile var cia1_ta_lo: u8
        fixed($dc05) volatile var cia1_ta_hi: u8
        fixed($dc0d) volatile var cia1_icr: u8
        fixed($dc0e) volatile var cia1_cra: u8
        fixed($d01a) volatile var vic_irq_mask: u8
        fixed($0900) volatile var ticks: u16
        fixed($0902) volatile var result: u32
        fixed($0906) volatile var isr_out: u16

        export function main(): u8
            var i: u32
            var sum: u32
            vic_irq_mask = 0
            cia1_icr = $7f
            ticks = 0
            cia1_ta_lo = $f4
            cia1_ta_hi = $01
            cia1_icr = $81
            cia1_cra = $11
            asm \"cli\"
            i = 1
            sum = 0
            while i != 3000
                sum = sum + i * 7919 / (i % 13 + 1)
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
            isr_out = ticks * 37 / 3 % 1000
            return
        end
        ",
    ) else {
        return;
    };
    let expected = (1u32..3000).fold(0u32, |sum, i| sum.wrapping_add(i * 7919 / (i % 13 + 1)));
    let ticks = u16::from_le_bytes([run.memory[0x0900], run.memory[0x0901]]);
    assert_eq!(run.exit_code, 0);
    assert!(ticks > 5, "expected timer interrupts, got {ticks}");
    assert_eq!(
        u32::from_le_bytes(run.memory[0x0902..0x0906].try_into().unwrap()),
        expected
    );
}

const WINDOW_MAIN: &str = "
module WindowMain
vector irq = tick
fixed($dc04) volatile var cia1_ta_lo: u8
fixed($dc05) volatile var cia1_ta_hi: u8
fixed($dc0d) volatile var cia1_icr: u8
fixed($dc0e) volatile var cia1_cra: u8
fixed($d01a) volatile var vic_irq_mask: u8
fixed($0b00) volatile var out: [12]u16

type Op = reentrant function(x: u16): u16

external reentrant function far_sum(a: u16, b: u16): u16
external function far_count(): u8
external procedure far_store(value: u16)
external function far_stored(): u16
external reentrant function far_fib(n: u16): u16
external reentrant function far_strlen_hello(): u16
external reentrant function far_apply(f: Op, x: u16): u16
external function far_handler(): Op
external procedure two_tick()
external function two_ticks(): u16

reentrant function triple(x: u16): u16
    return x * 3
end

export function main(): u8
    var c: u8
    var op: Op
    var i: u16
    var sum: u16
    out[0] = far_sum(3, 4)
    c = far_count()
    c = far_count()
    out[1] = u16(c)
    far_store($1234)
    out[2] = far_stored()
    out[3] = far_fib(9)
    out[4] = far_strlen_hello()
    op = triple
    out[5] = far_apply(op, 5)
    op = far_handler()
    out[6] = op(21)
    out[7] = far_apply(far_handler(), 7)

    vic_irq_mask = 0
    cia1_icr = $7f
    cia1_ta_lo = $f4
    cia1_ta_hi = $01
    cia1_icr = $81
    cia1_cra = $11
    asm \"cli\"
    i = 0
    sum = 0
    while i != 3000
        sum = far_sum(sum, i)
        i = i + 1
    end
    asm \"sei\"
    out[8] = sum
    out[9] = two_ticks()
    return 0
end

interrupt procedure tick()
    var ack: u8
    ack = cia1_icr
    two_tick()
    return
end
";

const WINDOW_ONE: &str = "
module WindowOne
var count: u8
var stored: u16

external reentrant function two_fib(n: u16): u16
external reentrant function two_strlen(text: near ^u8): u16

function one(): u16
    return 1
end

export reentrant function far_sum(a: u16, b: u16): u16
    return a + b + one()
end

export function far_count(): u8
    count = count + 1
    return count
end

export procedure far_store(value: u16)
    stored = value
    return
end

export function far_stored(): u16
    return stored
end

export reentrant function far_fib(n: u16): u16
    if n < 2
        return n
    end
    return two_fib(n - 1) + two_fib(n - 2)
end

export reentrant function far_strlen_hello(): u16
    return two_strlen(\"hello\")
end
";

const WINDOW_TWO: &str = "
module WindowTwo
type Op = reentrant function(x: u16): u16
var ticks: u16

external reentrant function far_fib(n: u16): u16

export reentrant function two_fib(n: u16): u16
    return far_fib(n)
end

export reentrant function two_strlen(text: near ^u8): u16
    var length: u16
    length = 0
    while text^ != 0
        length = length + 1
        text = text + 1
    end
    return length
end

export reentrant function far_apply(f: Op, x: u16): u16
    return f(x) + 1
end

reentrant function double(x: u16): u16
    return x + x
end

export function far_handler(): Op
    return double
end

export procedure two_tick()
    ticks = ticks + 1
    return
end

export function two_ticks(): u16
    return ticks
end
";

#[test]
fn window_banks_call_each_other_and_the_resident_kernel() {
    let module = |name: &str, source: &str, window| mega65::KernelModule {
        name: name.to_owned(),
        source: source.to_owned(),
        window,
    };
    let modules = mega65::compile_kernel_modules(&[
        module("main", WINDOW_MAIN, None),
        module("one", WINDOW_ONE, Some(1)),
        module("two", WINDOW_TWO, Some(2)),
    ])
    .expect("windowed modules should compile");
    let refs: Vec<(&str, &str)> = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect();
    let dir = temp_dir("window-banks");
    let rom = mega65::build_rom(&dir, &refs).expect("windowed ROM should link");
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping window banks: Xemu not found");
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).expect("Xemu run");
    assert_eq!(run.exit_code, 0);
    let word =
        |i: usize| u16::from_le_bytes([run.memory[0x0b00 + 2 * i], run.memory[0x0b01 + 2 * i]]);
    let expected_sum = (0u16..3000).fold(0u16, |sum, i| sum.wrapping_add(i).wrapping_add(1));
    assert_eq!(
        (0..9).map(word).collect::<Vec<_>>(),
        vec![8, 2, 0x1234, 34, 5, 16, 42, 15, expected_sum]
    );
    assert!(word(9) > 5, "expected timer interrupts, got {}", word(9));
}

#[test]
fn window_banks_reject_interrupt_handlers() {
    let error = mega65::compile_kernel_modules(&[mega65::KernelModule {
        name: "irq".to_owned(),
        source: "
            module Irq
            vector irq = tick
            interrupt procedure tick()
                return
            end
            "
        .to_owned(),
        window: Some(1),
    }])
    .expect_err("an interrupt vector cannot point into a window bank");
    assert!(error.contains("needs resident code"), "{error}");
}

/// A program larger than the old bank-0 program area that reads a file
/// into its own memory through GEMDOS: its BSS reaches into the program
/// bank's $8000-$BFFF part, so the read goes through the kernel's copies.
const BIG_PROGRAM: &str = "
module Big
const GEMDOS = $01
const CCONOUT = $0002
const FOPEN = $003d
const FCLOSE = $003e
const FREAD = $003f
const FCOUNT = $0102

var big: [40000]u8
var tail: [64]u8

procedure say(text: near ^u8)
    var ignored: u16
    while text^ != 0
        ignored = trap(GEMDOS, CCONOUT, u16(text^), 0)
        text = text + 1
    end
    return
end

export function main(): u8
    var handle: u16
    var got: u16
    var i: u16
    var ignored: u16
    big[0] = $5a
    big[39999] = $a5
    handle = trap(GEMDOS, FOPEN, \"DATA.TXT\", 0)
    ignored = trap(GEMDOS, FCOUNT, 64, 0)
    got = trap(GEMDOS, FREAD, handle, addr(tail))
    ignored = trap(GEMDOS, FCLOSE, handle, 0)
    i = 0
    while i != got
        ignored = trap(GEMDOS, CCONOUT, u16(tail[i]), 0)
        i = i + 1
    end
    if big[0] == $5a and big[39999] == $a5
        say(\"40000 bytes of BSS\\r\\n\")
    end
    return u8(got)
end
";

#[test]
fn hellord_runs_programs_in_their_own_bank() {
    let modules = mega65::compile_program_modules(&[("big".to_owned(), BIG_PROGRAM.to_owned())])
        .expect("program should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let program =
        mega65::build_program(&temp_dir("big-program"), &refs).expect("program should link");
    let (dir, rom) = hellord_rom("hellord-program-bank");
    let Some(xemu) = mega65::find_xemu() else {
        return;
    };
    if !mega65::xemu_supports_type_in(&xemu) {
        eprintln!("skipping: this Xemu build has no -typein option");
        return;
    }
    let data = b"Data read by a big program\r\n";
    let run = mega65::run_rom_with(
        &xemu,
        &dir,
        &rom,
        &mega65::XemuOptions {
            type_in: Some("S:\rBIG\rREBOOT\r".to_owned()),
            sd_files: vec![
                ("BIG.PRG".to_owned(), program),
                ("DATA.TXT".to_owned(), data.to_vec()),
            ],
            ..mega65::XemuOptions::default()
        },
    )
    .unwrap();
    assert!(!run.timed_out, "REBOOT should end the run:\n{}", run.screen);
    let expected = format!(
        "S> BIG\nData read by a big program\n40000 bytes of BSS\nExit status ${:02X}\n",
        data.len()
    );
    assert!(run.screen.contains(&expected), "screen:\n{}", run.screen);
    // The data landed in the program bank's $8000-$BFFF part (linear $48000).
    let upper = &run.memory[0x48000..0x4c000];
    assert!(
        upper.windows(data.len()).any(|window| window == data),
        "the file's bytes should be in the program bank"
    );
}
