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

use menschly::driver::{self, Target};
use menschly::mega65::{self, XemuRun};

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
fn constant_array_lengths_and_pointer_indexing() {
    let Some(run) = run_source(
        "pointer-index",
        "
        module PointerIndex
        const CYCLES = 6
        const PAIRS = CYCLES / 2
        type Pt = record(x: u8, y: u16)
        type PointIndexCheck = [PAIRS * 2 + 1]u8
        fixed($0830) volatile var out: [CYCLES + 1]u8
        fixed($0840) volatile var wout: [PAIRS]u16
        var cells: [CYCLES]u8
        var words: [PAIRS + 1]u16
        var pts: [PAIRS]Pt

        export function main(): u8
            var i: u8
            for i = 0 to CYCLES - 1
                cells[i] = i * 10
            end
            out[0] = third(addr(cells))
            out[1] = at(addr(cells), 4)
            bump(addr(cells), 5)
            out[2] = cells[5]
            words[2] = $1234
            wout[0] = word_at(addr(words), 2)
            pts[1].y = $beef
            pts[2].x = 9
            wout[1] = pts_y(addr(pts), 1)
            out[3] = pts_x(addr(pts), 2)
            out[4] = stack_at(addr(cells), 3)
            out[5] = u8(size(PointIndexCheck))
            out[6] = far_at($0830, 1)
            return CYCLES
        end

        function third(p: near ^u8): u8
            return p[2]
        end

        function at(p: near ^u8, i: u8): u8
            return p[i]
        end

        procedure bump(p: near ^u8, i: u8)
            p[i] = p[i] + 1
            return
        end

        function word_at(p: near ^u16, i: u16): u16
            return p[i]
        end

        function pts_y(p: near ^Pt, i: u8): u16
            return p[i].y
        end

        function pts_x(p: near ^Pt, i: u8): u8
            return p[i].x
        end

        stack function stack_at(p: near ^u8, i: u8): u8
            return p[i]
        end

        function far_at(p: far ^u8, i: u8): u8
            return p[i]
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 6);
    assert_eq!(&run.memory[0x0830..0x0837], &[20, 40, 51, 9, 30, 7, 40]);
    assert_eq!(&run.memory[0x0840..0x0844], &[0x34, 0x12, 0xef, 0xbe]);
}

#[test]
fn modules_share_exported_tables_and_variables() {
    // `Levels` lives in window bank 1: its code moves, its data stays
    // resident, so `main` reads and writes it directly.
    let sources = [
        mega65::KernelModule {
            name: "0-main".to_owned(),
            source: "
            module Main
            import Levels
            fixed($0850) volatile var out: [8]u8
            const PICKS: [1]near ^Spawn = [addr(SPAWNS)]

            export function main(): u8
                var i: u8
                var p: near ^Spawn
                out[0] = SPAWNS[1].x
                i = 2
                out[1] = SPAWNS[i].lives
                out[2] = u8(score >> 8)
                score = $4321
                out[3] = bump_score()
                p = PICKS[0]
                out[4] = p[1].y
                out[5] = SIZES[i]
                lamp = 9
                out[6] = lamp_seen()
                return u8(SPAWN_COUNT)
            end
            "
            .to_owned(),
            window: None,
        },
        mega65::KernelModule {
            name: "1-levels".to_owned(),
            source: "
            module Levels
            export const SPAWN_COUNT = 3
            export type Spawn = record(x: u8, y: u8, lives: u8)
            export const SPAWNS: [SPAWN_COUNT]Spawn = [(10, 20, 1), (30, 40, 2), (50, 60, 3)]
            export const SIZES: [4]u8 = [1, 2, 4, 8]
            export var score: u16 = $1234
            export fixed($0860) volatile var lamp: u8

            export function bump_score(): u8
                score = score + 1
                return u8(score)
            end

            export function lamp_seen(): u8
                return lamp + 1
            end
            "
            .to_owned(),
            window: Some(1),
        },
    ];
    let modules = mega65::compile_kernel_modules(&sources).expect("modules should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir("shared-data");
    let rom = mega65::build_rom(&dir, &refs)
        .unwrap_or_else(|error| panic!("modules should link: {error}"));
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping shared-data: Xemu not found (set MENSCH_XMEGA65)");
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30))
        .unwrap_or_else(|error| panic!("shared-data: {error}"));
    assert_eq!(run.exit_code, 3);
    assert_eq!(&run.memory[0x0850..0x0857], &[30, 3, 0x12, 0x22, 40, 4, 10]);
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
/// `hellord/boot-mega65.sh`).
const HELLORD_MEGA65_MODULES: [(&str, Option<u8>); 10] = [
    ("os", None),
    ("dos", Some(1)),
    ("fat", Some(2)),
    ("exec", Some(2)),
    ("console", None),
    ("files", None),
    ("machines/mega65", None),
    ("bios", None),
    ("gemdos", None),
    ("traps-mega65", None),
];

fn hellord_rom(name: &str) -> (PathBuf, Vec<u8>) {
    let hellord = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hellord");
    let sources = HELLORD_MEGA65_MODULES
        .iter()
        .enumerate()
        .map(|(index, (module, window))| mega65::KernelModule {
            name: format!("{index}-{}", module.replace('/', "-")),
            source: std::fs::read_to_string(hellord.join(format!("{module}.mensch")))
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
    let hellord = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hellord");
    let machine = std::fs::read_to_string(hellord.join("machines/mega65.mensch")).unwrap();
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
    let hellord = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hellord");
    let mut sources = vec![mega65::KernelModule {
        name: "0-caller".to_owned(),
        source: caller.to_owned(),
        window: None,
    }];
    for (index, (module, window)) in HELLORD_MEGA65_MODULES.iter().enumerate() {
        if *module == "os" {
            continue;
        }
        sources.push(mega65::KernelModule {
            name: format!("{}-{}", index + 1, module.replace('/', "-")),
            source: std::fs::read_to_string(hellord.join(format!("{module}.mensch")))
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
    // Xemu builds the card from a host directory, so the entries come in
    // whatever order the host's file system lists them.
    let entries = ["HELLO.TXT    $0026\n", "BIG.DAT      $2710\n"];
    let expected = |first: &str, second: &str| {
        format!(
            "S> DIR\nDirectory of S:/\n{first}{second}02 file(s)  00 dir(s)\nS> TYPE hello.txt\nHello from the SD card!\nSecond line.\nS> TYPE NOPE.TXT\nFile not found\n"
        )
    };
    assert!(
        run.screen.contains(&expected(entries[0], entries[1]))
            || run.screen.contains(&expected(entries[1], entries[0])),
        "screen:\n{}",
        run.screen
    );
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

const DATA_AND_PLACES: &str = "
module DataAndPlaces
fixed($0c00) volatile var out: [32]u8
fixed($0c40) volatile var wout: [8]u16

type Point = record(x: u8, y: u8)
type Actor = record(pos: Point, speed: i8, name: near ^u8)
type Op = function(): u8

const COUNT = 3
const LAST = COUNT - 1
const SQUARES: [6]u8 = [0, 1, 4, 9, 16, 25]
const WIDE: [3]u16 = [$1234, -2, LAST * 1000]
const GRID: [2][3]u8 = [[1, 2, 3], [4, 5, 6]]
const NAMES: [2]near ^u8 = [\"alpha\", \"beta\"]
const TITLE: [8]u8 = \"MENSCH\"
const OPS: [2]Op = [seven, nine]

var actors: [3]Actor = [
    ((1, 2), 1, \"ant\"),
    ((10, 20), -1, \"bee\"),
    ((100, 200), 2, \"cat\"),
]
var counter: u16 = 500

function seven(): u8
    return 7
end

function nine(): u8
    return 9
end

procedure nudge(actor: near ^Actor)
    actor^.pos.x = actor^.pos.x + u8(actor^.speed)
    return
end

export function main(): u8
    var i: u8
    var j: u16
    var k: u8
    var sum: u16
    var p: near ^Actor
    var op: Op
    var name: near ^u8

    # Constant tables, indexed by u8 and u16 variables.
    i = 4
    out[0] = SQUARES[i]
    j = 5
    out[1] = SQUARES[j]
    wout[0] = WIDE[1]
    wout[1] = WIDE[LAST]
    i = 1
    k = 2
    out[2] = GRID[i][k]
    name = NAMES[i]
    out[3] = name^
    out[4] = TITLE[5]
    out[5] = TITLE[6]
    i = 1
    op = OPS[i]
    out[6] = op()

    # Arrays of records: element fields, through pointers too.
    i = 2
    out[7] = actors[i].pos.y
    actors[i].pos.y = 77
    out[8] = actors[2].pos.y
    p = addr(actors)
    nudge(p)
    out[9] = actors[0].pos.x
    p = p + size(Actor)
    nudge(p)
    out[10] = actors[1].pos.x
    name = actors[1].name
    out[11] = name^

    out[20] = size(Actor)
    out[21] = size([4]Point)

    # for loops (inclusive, and to the type's limit).
    sum = 0
    for i = 1 to 10
        sum = sum + u16(i)
    end
    wout[2] = sum
    sum = 0
    for i = 250 to 255
        sum = sum + 1
    end
    wout[3] = sum
    k = 0
    for i = 3 downto 0
        out[12 + u16(k)] = i
        k = k + 1
    end
    sum = 0
    for i = 5 to 4
        sum = sum + 1
    end
    wout[4] = sum

    # Conditions: not, grouping, bare values.
    out[16] = 0
    out[17] = 0
    out[18] = 0
    i = 3
    if not (i == 2 or i == 4)
        out[16] = 1
    end
    if (i > 1 and i < 5) and not i == 9
        out[17] = 1
    end
    if i
        out[18] = 1
    end
    if (i + 1) == 4
        out[19] = 1
    else
        out[19] = 0
    end

    counter = counter + 1
    wout[5] = counter
    return 0
end
";

#[test]
fn data_tables_record_elements_for_loops_and_conditions() {
    let Some(run) = run_source("data-and-places", DATA_AND_PLACES) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    let words: Vec<u16> = (0..6)
        .map(|i| u16::from_le_bytes([run.memory[0x0c40 + 2 * i], run.memory[0x0c41 + 2 * i]]))
        .collect();
    assert_eq!(words, vec![0xfffe, 2000, 55, 6, 0, 501]);
    assert_eq!(
        &run.memory[0x0c00..0x0c16],
        &[16, 25, 6, b'b', b'H', 0, 9, 200, 77, 2, 9, b'b', 3, 2, 1, 0, 1, 1, 1, 1, 5, 8]
    );
}

#[test]
fn constant_tables_reject_assignment_and_bad_initialisers() {
    let compile = |source: &str| {
        driver::compile_source(source, Target::Mega65, driver::DEFAULT_DIRECT_PAGE_BUDGET)
            .err()
            .unwrap_or_default()
    };
    let error = compile(
        "
        module Bad
        const TABLE: [2]u8 = [1, 2]
        export function main(): u8
            TABLE[0] = 3
            return 0
        end
        ",
    );
    assert!(error.contains("cannot be assigned"), "{error}");
    let error = compile(
        "
        module Bad
        const TABLE: [3]u8 = [1, 2]
        export function main(): u8
            return 0
        end
        ",
    );
    assert!(
        error.contains("2 elements for a 3-element array"),
        "{error}"
    );
    let error = compile(
        "
        module Bad
        var table: [2]u8
        export function main(): u8
            table[2] = 1
            return 0
        end
        ",
    );
    assert!(error.contains("outside the 2-element array"), "{error}");
}

#[test]
fn parallel_arrays_store_each_field_as_a_column() {
    let Some(run) = run_source(
        "parallel-arrays",
        "
        module Parallel
        fixed($0d00) volatile var out: [8]u8
        fixed($0d10) volatile var wout: [2]u16

        type Point = record(x: u8, y: u8)
        type Actor = record(pos: Point, dx: i8, score: u16)

        var actors: parallel [4]Actor = [
            ((1, 2), 1, 100),
            ((10, 20), -1, 200),
            ((30, 40), 2, 300),
            ((50, 60), 3, 400),
        ]

        export function main(): u8
            var i: u8
            for i = 0 to 3
                actors[i].pos.x = actors[i].pos.x + u8(actors[i].dx)
                actors[i].score = actors[i].score + 1
            end
            i = 2
            out[0] = actors[i].pos.x
            out[1] = actors[1].pos.x
            out[2] = actors[3].pos.y
            wout[0] = actors[i].score
            wout[1] = actors[0].score
            return 0
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    assert_eq!(&run.memory[0x0d00..0x0d03], &[32, 9, 60]);
    let word = |at: usize| u16::from_le_bytes([run.memory[at], run.memory[at + 1]]);
    assert_eq!((word(0x0d10), word(0x0d12)), (301, 101));
}

#[test]
fn constant_shifts_of_every_count_and_width() {
    // Every constant shift of 8-, 16- and 32-bit values, signed and
    // unsigned, against Rust's own shifts.
    let cases: [(&str, u32, u32); 6] = [
        ("u8", 8, 0xb5),
        ("i8", 8, 0xb5),
        ("u16", 16, 0xbcde),
        ("i16", 16, 0xbcde),
        ("u32", 32, 0x89ab_cdef),
        ("i32", 32, 0x89ab_cdef),
    ];
    let mut source = String::from("module Shifts\n");
    let mut expected = Vec::new();
    let mut at = 0x1000u32;
    for (ty, bits, value) in cases {
        source.push_str(&format!("var v_{ty}: {ty} = ${value:x}\n"));
        let bytes = bits / 8;
        for count in 1..bits {
            for left in [true, false] {
                let op = if left { "<<" } else { ">>" };
                source.push_str(&format!(
                    "fixed(${at:x}) volatile var r_{ty}_{}_{count}: {ty}\n",
                    if left { "l" } else { "r" }
                ));
                let result = match (ty.starts_with('i'), left) {
                    (_, true) => (u64::from(value) << count) as u32,
                    (false, false) => value >> count,
                    (true, false) => {
                        let signed = (value << (32 - bits)) as i32 >> (32 - bits);
                        (signed >> count) as u32
                    }
                };
                expected.push((at, bytes, result, format!("{ty} {op} {count}")));
                at += bytes;
            }
        }
    }
    source.push_str("export function main(): u8\n");
    for (ty, bits, _) in cases {
        for count in 1..bits {
            source.push_str(&format!("    r_{ty}_l_{count} = v_{ty} << {count}\n"));
            source.push_str(&format!("    r_{ty}_r_{count} = v_{ty} >> {count}\n"));
        }
    }
    source.push_str("    return 0\nend\n");
    let Some(run) = run_source("shifts", &source) else {
        return;
    };
    for (at, bytes, result, what) in expected {
        let at = at as usize;
        let got = (0..bytes as usize)
            .map(|i| u32::from(run.memory[at + i]) << (8 * i))
            .sum::<u32>();
        let mask = if bytes == 4 {
            u32::MAX
        } else {
            (1 << (8 * bytes)) - 1
        };
        assert_eq!(got, result & mask, "{what}");
    }
}

#[test]
fn multiplication_by_two_bit_constants() {
    // Constants with two bits set multiply by two shifts and an add; check
    // them against Rust's wrapping products, in place too.
    let constants = [3u32, 5, 6, 9, 10, 12, 17, 24, 40, 48, 96, 130, 160, 192];
    let cases: [(&str, u32, u32); 3] = [("u8", 8, 0xb5), ("u16", 16, 0x9c37), ("i16", 16, 0xfed4)];
    let mut source = String::from("module Multiply\n");
    let mut expected = Vec::new();
    let mut at = 0x1000u32;
    for (ty, bits, value) in cases {
        source.push_str(&format!("var v_{ty}: {ty} = ${value:x}\n"));
        for c in constants {
            if c >= (1 << bits) {
                continue;
            }
            for in_place in [false, true] {
                let name = format!("r_{ty}_{c}_{}", if in_place { "p" } else { "e" });
                source.push_str(&format!("fixed(${at:x}) volatile var {name}: {ty}\n"));
                let mask = (1u64 << bits) - 1;
                let product = (u64::from(value) * u64::from(c)) & mask;
                expected.push((
                    at,
                    bits / 8,
                    product as u32,
                    format!("{ty} * {c}{}", if in_place { " in place" } else { "" }),
                ));
                at += bits / 8;
            }
        }
    }
    source.push_str("export function main(): u8\n");
    for (ty, bits, _) in cases {
        source.push_str(&format!("    var w_{ty}: {ty}\n"));
        let _ = bits;
    }
    for (ty, bits, _) in cases {
        for c in constants {
            if c >= (1 << bits) {
                continue;
            }
            source.push_str(&format!("    r_{ty}_{c}_e = v_{ty} * {c}\n"));
            source.push_str(&format!(
                "    w_{ty} = v_{ty}\n    w_{ty} = w_{ty} * {c}\n    r_{ty}_{c}_p = w_{ty}\n"
            ));
        }
    }
    source.push_str("    return 0\nend\n");
    let Some(run) = run_source("multiply", &source) else {
        return;
    };
    for (at, bytes, product, what) in expected {
        let at = at as usize;
        let got = (0..bytes as usize)
            .map(|i| u32::from(run.memory[at + i]) << (8 * i))
            .sum::<u32>();
        assert_eq!(got, product, "{what}");
    }
}

#[test]
fn unsigned_multiply_divide_and_remainder_at_every_width() {
    // Products, quotients and remainders of variables (so nothing folds),
    // against Rust's: top bits set, a divisor bigger than the dividend,
    // and dividing by zero (a quotient of all ones, a remainder of the
    // dividend).
    let pairs: [(u32, u32); 6] = [
        (0xffff_fff1, 7),
        (0x8765_4321, 0x1234),
        (1_000_000_007, 65_521),
        (12, 0xfedc_ba98),
        (0xdead_beef, 0),
        (0x0100_0000, 0x0000_0101),
    ];
    let types: [(&str, u32); 4] = [("u8", 1), ("u16", 2), ("u24", 3), ("u32", 4)];
    let mut source = String::from("module UnsignedMath\n");
    // Each case twice: in straight-line code and inside a loop, which
    // are compiled differently.
    let mut body = String::new();
    let mut looped = String::new();
    let mut expected = Vec::new();
    let mut at = 0x1000u32;
    for (ty, bytes) in types {
        let mask = if bytes == 4 { u32::MAX } else { (1u32 << (8 * bytes)) - 1 };
        for (k, (a, b)) in pairs.iter().enumerate() {
            let (a, b) = (a & mask, b & mask);
            source.push_str(&format!("var a_{ty}_{k}: {ty} = ${a:x}\nvar b_{ty}_{k}: {ty} = ${b:x}\n"));
            let results = [
                ("*", a.wrapping_mul(b) & mask),
                ("/", if b == 0 { mask } else { a / b }),
                ("%", if b == 0 { a } else { a % b }),
            ];
            for (op, value) in results {
                for (code, place) in [(&mut body, "straight"), (&mut looped, "in a loop")] {
                    source.push_str(&format!("fixed(${at:x}) volatile var r_{at:x}: {ty}\n"));
                    code.push_str(&format!("    r_{at:x} = a_{ty}_{k} {op} b_{ty}_{k}\n"));
                    expected.push((at, bytes, value, format!("{ty} ${a:x} {op} ${b:x}, {place}")));
                    at += bytes;
                }
            }
        }
    }
    source.push_str("export function main(): u8\n");
    source.push_str("    var pass: u8\n");
    source.push_str(&body);
    source.push_str("    pass = 0\n    while pass != 1\n");
    source.push_str(&looped);
    source.push_str("        pass = pass + 1\n    end\n    return 0\nend\n");
    let Some(run) = run_source("unsigned-math", &source) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    for (at, bytes, value, what) in expected {
        let at = at as usize;
        let got = (0..bytes as usize)
            .map(|i| u32::from(run.memory[at + i]) << (8 * i))
            .sum::<u32>();
        assert_eq!(got, value, "{what}");
    }
}

#[test]
fn signed_divide_and_remainder_at_every_width() {
    // Quotients truncate toward zero and remainders take the dividend's
    // sign, for every pairing of signs, the most negative value over -1,
    // and constant divisors of either sign. Dividing by zero gives a
    // quotient of all ones, negated for a negative dividend, and a
    // remainder of the dividend.
    let pairs: [(i64, i64); 8] = [
        (100, 7),
        (-100, 7),
        (100, -7),
        (-100, -7),
        (i64::MIN, -1),
        (-5, 0),
        (5, 0),
        (-1_000_003, 977),
    ];
    let types: [(&str, u32); 3] = [("i8", 1), ("i16", 2), ("i32", 4)];
    let mut source = String::from("module SignedMath\n");
    // Each case twice: in straight-line code and inside a loop, which
    // are compiled differently.
    let mut body = String::new();
    let mut looped = String::new();
    let mut variables = String::new();
    let mut expected = Vec::new();
    let mut at = 0x1000u32;
    for (ty, bytes) in types {
        let bits = 8 * bytes;
        let wrap = |v: i64| (v << (64 - bits)) >> (64 - bits);
        let mask = if bytes == 4 { u32::MAX } else { (1u32 << bits) - 1 };
        let mut check = |lhs: String, rhs: String, a: i64, b: i64, what: String| {
            let quotient = match b {
                0 if a < 0 => 1,
                0 => -1,
                _ => wrap(a.wrapping_div(b)),
            };
            let remainder = if b == 0 { a } else { wrap(a.wrapping_rem(b)) };
            for (op, value) in [("/", quotient), ("%", remainder)] {
                for (code, place) in [&mut body, &mut looped].into_iter().zip(["straight", "in a loop"]) {
                    source.push_str(&format!("fixed(${at:x}) volatile var r_{at:x}: {ty}\n"));
                    code.push_str(&format!("    r_{at:x} = {lhs} {op} {rhs}\n"));
                    expected.push((at, bytes, value as u32 & mask, format!("{what} {op}, {place}")));
                    at += bytes;
                }
            }
        };
        for (k, (a, b)) in pairs.iter().enumerate() {
            let (a, b) = (wrap(*a), wrap(*b));
            let hex = |v: i64| format!("${:x}", v as u32 & mask);
            variables.push_str(&format!("var a_{ty}_{k}: {ty} = {}\nvar b_{ty}_{k}: {ty} = {}\n", hex(a), hex(b)));
            check(format!("a_{ty}_{k}"), format!("b_{ty}_{k}"), a, b, format!("{ty} {a} by {b}"));
            for c in [7i64, -7] {
                check(format!("a_{ty}_{k}"), format!("{c}"), a, c, format!("{ty} {a} by constant {c}"));
            }
        }
    }
    source.push_str(&variables);
    source.push_str("export function main(): u8\n");
    source.push_str("    var pass: u8\n");
    source.push_str(&body);
    source.push_str("    pass = 0\n    while pass != 1\n");
    source.push_str(&looped);
    source.push_str("        pass = pass + 1\n    end\n    return 0\nend\n");
    let Some(run) = run_source("signed-math", &source) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    for (at, bytes, value, what) in expected {
        let at = at as usize;
        let got = (0..bytes as usize)
            .map(|i| u32::from(run.memory[at + i]) << (8 * i))
            .sum::<u32>();
        assert_eq!(got, value, "{what}");
    }
}

#[test]
fn parallel_arrays_past_256_bytes_index_by_variable() {
    // `wide`'s column starts 200 bytes in and `cell`'s 600: their element
    // addresses are formed from symbol + offset beyond a byte.
    let Some(run) = run_source(
        "parallel-wide",
        "
        module ParallelWide
        type Item = record(tag: u8, wide: u16, cell: u16)
        var items: parallel [200]Item
        fixed($0870) volatile var out: [4]u8

        export function main(): u8
            var i: u8
            i = 199
            items[i].cell = $1234
            items[i].wide = $5678
            out[0] = u8(items[i].cell)
            out[1] = u8(items[i].cell >> 8)
            out[2] = u8(items[i].wide)
            out[3] = u8(items[i].wide >> 8)
            return 0
        end
        ",
    ) else {
        return;
    };
    assert_eq!(&run.memory[0x0870..0x0874], &[0x34, 0x12, 0x78, 0x56]);
}

#[test]
fn parallel_array_elements_are_only_reached_through_fields() {
    let error = driver::compile_source(
        "
        module Bad
        type Point = record(x: u8, y: u8)
        var points: parallel [4]Point
        export function main(): u8
            return points[1][0]
        end
        ",
        Target::Mega65,
        driver::DEFAULT_DIRECT_PAGE_BUDGET,
    )
    .err()
    .unwrap_or_default();
    assert!(error.contains("parallel"), "{error}");
}

#[test]
fn stack_routines_take_stack_arguments_with_static_locals() {
    let Some(run) = run_source(
        "stack-routines",
        "
        module StackRoutines
        fixed($0e00) volatile var out: [3]u16
        type Combine = stack function(a: u16, b: u8): u16

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

        export function main(): u8
            var combine: Combine
            out[0] = spread(1000, 5, 2000, 7)
            combine = weigh
            out[1] = combine(300, 4)
            out[2] = bump(21)
            return 0
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    let word =
        |i: usize| u16::from_le_bytes([run.memory[0x0e00 + 2 * i], run.memory[0x0e01 + 2 * i]]);
    // weigh(a, b) = a + 3b.
    assert_eq!((word(0), word(1), word(2)), (1015 + 2021, 312, 42));
}

#[test]
fn nested_calls_in_arguments_do_not_clobber_earlier_arguments() {
    let Some(run) = run_source(
        "argument-staging",
        "
        module Staging
        fixed($0e10) volatile var out: [2]u8
        var count: u8

        function g(b: u8): u8
            count = count + 1
            return b + 1
        end

        function f(a: u8, c: u8): u8
            return a * 10 + c
        end

        export function main(): u8
            count = 4
            out[0] = f(1, g(2))
            out[1] = f(count, g(count))
            return 0
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    // Arguments evaluate left to right: count (5 after the first line) is
    // read before g(count) makes it 6.
    assert_eq!(&run.memory[0x0e10..0x0e12], &[13, 56]);
}

#[test]
fn register_arguments_and_carry_results() {
    let source = "
        module Registers
        fixed($0e10) volatile var out: [8]u8
        var table: [4]u8 = [10, 20, 30, 40]
        var total: u16

        # A byte in each register, and a word in X (low) and Y.
        function pick(index: u8 in x, bias: u8 in a): u8
            return table[index] + bias
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

        export function main(): u8
            out[0] = pick(2, 5)
            out[1] = pick(u8(out[0] - 32), pick(0, 1))
            total = 0
            add($1234, 2)
            add($0101, out[0])
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
            return 0
        end
        ";
    let Some(run) = run_source("register-arguments", source) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    // 30+5=35; table[3]+11 = 51; $1234+2+$0101+35 = $135a.
    assert_eq!(
        &run.memory[0x0e10..0x0e18],
        &[35, 51, 0x5a, 0x13, 1, 0, 1, 1]
    );
}

#[test]
fn self_referential_records_field_calls_and_element_addresses() {
    let Some(run) = run_source(
        "language-gaps",
        "
        module Gaps
        fixed($0e10) volatile var out: [8]u8

        type Node = record(value: u8, next: near ^Node)
        var nodes: [4]Node

        type Handler = stack function(arg: u8): u8
        type Command = record(name: [4]u8, run: Handler)
        var commands: [2]Command

        stack function double(arg: u8): u8
            return arg + arg
        end

        stack function negate(arg: u8): u8
            return 0 - arg
        end

        reentrant function total(node: near ^Node): u8
            if not node
                return 0
            end
            return node^.value + total(node^.next)
        end

        function first_letter(name: near ^u8): u8
            return name^
        end

        export function main(): u8
            var i: u8
            var p: near ^Node
            for i = 0 to 3
                nodes[i].value = i * 10 + 1
                nodes[i].next = 0
            end
            nodes[0].next = addr(nodes[2])
            nodes[2].next = addr(nodes[3])
            out[0] = total(addr(nodes[0]))
            p = addr(nodes[1])
            p^.next = addr(nodes[3])
            out[1] = p^.next^.value

            commands[0].name[0] = 'D'
            commands[0].run = double
            commands[1].name[0] = 'N'
            commands[1].run = negate
            i = 1
            out[2] = commands[0].run(7)
            out[3] = commands[i].run(5)
            out[4] = first_letter(addr(commands[i].name))
            out[5] = first_letter(addr(commands[0].name[0]))
            out[6] = commands[i - 1].run(commands[i].run(2))
            return 0
        end
        ",
    ) else {
        return;
    };
    assert_eq!(run.exit_code, 0);
    // 1 + 21 + 31 = 53; node 1 -> node 3; 14; -5; 'N'; 'D'; -4.
    assert_eq!(
        &run.memory[0x0e10..0x0e17],
        &[53, 31, 14, 251, b'N', b'D', 252]
    );
}

fn vic4_rom(name: &str, caller: &str) -> (PathBuf, Vec<u8>) {
    library_rom(name, caller, &["vic4"])
}

/// Builds `caller` with the named modules of `lib/` into a ROM.
fn library_rom(name: &str, caller: &str, libraries: &[&str]) -> (PathBuf, Vec<u8>) {
    let lib = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib");
    let mut sources = vec![mega65::KernelModule {
        name: "0-caller".to_owned(),
        source: caller.to_owned(),
        window: None,
    }];
    for (index, library) in libraries.iter().enumerate() {
        sources.push(mega65::KernelModule {
            name: format!("{}-{library}", index + 1),
            source: std::fs::read_to_string(lib.join(format!("{library}.mensch")))
                .expect("library should be readable"),
            window: None,
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

#[test]
fn vic4_library_sets_up_the_screen_dma_palette_raster_and_sprites() {
    let (dir, rom) = vic4_rom(
        "vic4-library",
        "
        module Vic4Test
        vector irq = frame
        import Vic4
        fixed($0e00) volatile var out: [32]u8
        fixed($0e20) volatile var lines: [4]u16
        fixed($0e28) volatile var frames: u16
        fixed($d054) volatile var vic4_control: u8
        fixed($d058) volatile var line_step: u16
        fixed($d05e) volatile var char_count: u8
        fixed($d060) volatile var screen_pointer: [4]u8
        fixed($d06c) volatile var sprite_pointer_address: [3]u8
        fixed($d114) volatile var red_20: u8
        fixed($d214) volatile var green_20: u8
        fixed($d000) volatile var sprite_xy: [4]u8
        fixed($d010) volatile var sprite_x_high: u8
        fixed($d070) volatile var palette_select: u8
        fixed($d055) volatile var extended_height: u8
        fixed($d056) volatile var sprite_height: u8
        fixed($d057) volatile var extended_width: u8
        fixed($d06b) volatile var sixteen_colour: u8
        fixed($d017) volatile var tall: u8
        fixed($d01d) volatile var wide: u8
        fixed($d01b) volatile var behind: u8
        fixed($d100) volatile var red: [256]u8
        fixed($d300) volatile var blue: [256]u8
        fixed($0e30) volatile var sprite_out: [12]u8
        fixed($d048) volatile var top_border: u16
        fixed($d04a) volatile var bottom_border: u16
        fixed($d04e) volatile var text_top: u16
        fixed($d07b) volatile var display_rows: u8
        fixed($0e40) volatile var rows_out: [6]u16
        fixed($0e4c) volatile var rows_count: u8

        const PATTERN: [6]u8 = [1, 2, 3, 4, 5, 6]
        const COLOURS: [48]u8 = [
            0, 0, 0, $21, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, $f0
        ]

        export function main(): u8
            vic4_init()
            dma_fill($40000, 0, 0)
            dma_fill($50000, 0, 0)
            dma_fill($40000, $5a, 300)
            dma_fill_step($40200, 7, 4, 2)
            dma_copy(u32(addr(PATTERN)), $41000, 6)
            vic4_text16($50000, 40, 0)
            vic4_fill_screen($1234, 1000)
            vic4_put(3, 2, $1005)
            vic4_palette_set(20, $12, $80, $ff)
            vic4_sprite_table($45000)
            vic4_sprite_image(1, $46040)
            vic4_sprite_place(1, 300, 77)
            out[0] = screen_pointer[0]
            out[1] = screen_pointer[1]
            out[2] = screen_pointer[2]
            out[3] = screen_pointer[3] & $0f
            out[4] = u8(line_step)
            out[5] = char_count
            out[6] = vic4_control & $05
            out[7] = red_20
            out[8] = green_20
            out[9] = sprite_pointer_address[0]
            out[10] = sprite_pointer_address[1]
            out[11] = sprite_pointer_address[2]
            out[12] = sprite_xy[2]
            out[13] = sprite_xy[3]
            out[14] = sprite_x_high & 2
            out[15] = u8(vic4_tile_at($40040))
            out[16] = u8(vic4_tile_at($40040) >> 8)
            rows_out[0] = top_border & $0fff
            rows_out[1] = bottom_border & $0fff
            rows_out[2] = text_top & $0fff
            vic4_text_rows(26)
            rows_out[3] = top_border & $0fff
            rows_out[4] = bottom_border & $0fff
            rows_out[5] = text_top & $0fff
            rows_count = display_rows
            red[17] = 0
            vic4_sprite_palette(1)
            vic4_sprite_colours(1, addr(COLOURS))
            vic4_sprites_16_colour($06)
            vic4_sprite_height(24, $02)
            vic4_sprites_double($04, $08)
            vic4_sprites_behind($10)
            sprite_out[0] = palette_select
            sprite_out[1] = red[17]
            vic4_palette_edit(1)
            sprite_out[2] = red[17]
            sprite_out[3] = blue[31]
            vic4_palette_edit(0)
            sprite_out[4] = sixteen_colour
            sprite_out[5] = extended_height
            sprite_out[6] = sprite_height
            sprite_out[7] = wide
            sprite_out[8] = tall
            sprite_out[9] = behind
            sprite_out[10] = palette_select
            sprite_out[11] = extended_width
            vic4_wait_line(200)
            lines[0] = vic4_raster()
            vic4_wait_line(280)
            lines[1] = vic4_raster()
            frames = 0
            vic4_raster_irq(100)
            asm \"cli\"
            vic4_wait_line(50)
            vic4_wait_line(50)
            vic4_wait_line(50)
            asm \"sei\"
            vic4_raster_irq_off()
            return 0
        end

        interrupt procedure frame()
            frames = frames + 1
            vic4_raster_irq(100)
            return
        end
        ",
    );
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping vic4-library: Xemu not found (set MENSCH_XMEGA65)");
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    assert_eq!(run.exit_code, 0);
    let memory = &run.memory;
    assert!(memory[0x40000..0x4012c].iter().all(|&byte| byte == 0x5a));
    assert_eq!(memory[0x4012c], 0);
    assert_eq!(&memory[0x40200..0x40208], &[7, 0, 7, 0, 7, 0, 7, 0]);
    assert_eq!(&memory[0x41000..0x41006], &[1, 2, 3, 4, 5, 6]);
    // The screen: 1000 cells of tile $1234, then (3, 2) = $1005.
    assert_eq!(&memory[0x50000..0x50004], &[0x34, 0x12, 0x34, 0x12]);
    assert_eq!(&memory[0x507ce..0x507d0], &[0x34, 0x12]);
    assert_eq!(&memory[0x507d0..0x507d2], &[0, 0]);
    let cell = 0x50000 + 2 * 80 + 3 * 2;
    assert_eq!(&memory[cell..cell + 2], &[0x05, 0x10]);
    // Sprite 1's pointer: $46040 / 64.
    assert_eq!(&memory[0x45002..0x45004], &[0x81, 0x11]);
    let out = &memory[0x0e00..0x0e11];
    assert_eq!(
        out,
        &[
            0x00, 0x00, 0x05, 0x00, 80, 40, 0x05, 0x21, 0x08, 0x00, 0x50, 0x84, 44, 77, 2, 0x01,
            0x10
        ]
    );
    let line = |i: usize| u16::from_le_bytes([memory[0x0e20 + 2 * i], memory[0x0e21 + 2 * i]]);
    assert!((200..204).contains(&line(0)), "raster {}", line(0));
    assert!((280..284).contains(&line(1)), "raster {}", line(1));
    // Sprite 1's colours went to entries 16-31 of palette bank 1 (nybbles
    // swapped), leaving bank 0 alone; the edited bank is back to 0.
    assert_eq!(
        &memory[0x0e30..0x0e3c],
        &[0x04, 0x00, 0x12, 0x0f, 0x06, 0x02, 24, 0x04, 0x08, 0x10, 0x04, 0x06]
    );
    let frames = u16::from_le_bytes([memory[0x0e28], memory[0x0e29]]);
    assert!((2..=3).contains(&frames), "{frames} raster interrupts");
    // 26 rows: 416 physical lines between the borders, around the same
    // centre, with the text starting as far below the top border as before.
    let rows = |i: usize| {
        i32::from(u16::from_le_bytes([
            memory[0x0e40 + 2 * i],
            memory[0x0e41 + 2 * i],
        ]))
    };
    let (top, bottom, text) = (rows(0), rows(1), rows(2));
    let (new_top, new_bottom, new_text) = (rows(3), rows(4), rows(5));
    assert_eq!(
        new_bottom - new_top,
        416,
        "{:?}",
        (top, bottom, new_top, new_bottom)
    );
    assert_eq!(new_text - new_top, text - top);
    assert!(((new_top + new_bottom) - (top + bottom)).abs() <= 1);
    assert_eq!(memory[0x0e4c], 25);
}

#[test]
fn vic4_demo_draws_its_room_and_sets_up_its_sprite() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let read = |path: &str| std::fs::read_to_string(root.join(path)).expect("source");
    let sources = [
        mega65::KernelModule {
            name: "0-demo".to_owned(),
            source: read("examples/vic4-demo.mensch"),
            window: None,
        },
        mega65::KernelModule {
            name: "1-vic4".to_owned(),
            source: read("lib/vic4.mensch"),
            window: None,
        },
    ];
    let modules = mega65::compile_kernel_modules(&sources).expect("demo should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir("vic4-demo");
    let rom = mega65::build_rom(&dir, &refs).expect("demo should link");
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping vic4-demo: Xemu not found (set MENSCH_XMEGA65)");
        return;
    };
    let run = mega65::run_rom_with(
        &xemu,
        &dir,
        &rom,
        &mega65::XemuOptions {
            timeout: Duration::from_secs(2),
            stop_at_timeout: true,
            ..mega65::XemuOptions::default()
        },
    )
    .unwrap();
    assert!(run.timed_out, "the demo runs until stopped");
    let memory = &run.memory;
    let cell = |column: usize, row: usize| {
        let at = 0x50000 + (row * 40 + column) * 2;
        u16::from_le_bytes([memory[at], memory[at + 1]])
    };
    // Tiles $1000 (empty), $1001 (wall), $1002 (ledge) at $40000.
    assert_eq!(cell(0, 0), 0x1001);
    assert_eq!(cell(39, 12), 0x1001);
    assert_eq!(cell(20, 24), 0x1001);
    assert_eq!(cell(20, 5), 0x1000);
    assert_eq!(cell(4, 19), 0x1002);
    assert_eq!(cell(29, 14), 0x1002);
    assert!(memory[0x40000..0x40040].iter().all(|&pixel| pixel == 0));
    assert_eq!(memory[0x40040], 48, "the wall's top row is mortar");
    assert_eq!(memory[0x40080], 56, "the ledge's top row is grass");
    // The hero at $46000 and the wisp at $46080 (pointers / 64).
    assert_eq!(&memory[0x45000..0x45004], &[0x80, 0x11, 0x82, 0x11]);
    assert_eq!(&memory[0x46000..0x46008], &[0, 0, 0, 0x11, 0x11, 0, 0, 0]);
    assert_eq!(&memory[0x46080..0x46088], &[0, 0, 0, 0x01, 0x10, 0, 0, 0]);
}

#[test]
fn actors_draw_hardware_sprites_and_raster_rewrite_characters() {
    let (dir, rom) = library_rom(
        "actors",
        "
        module ActorsTest
        import Vic4
        import Actors
        fixed($0e00) volatile var out: [8]u8
        fixed($d000) volatile var sprite_xy: [2]u8
        fixed($d015) volatile var sprite_enable: u8
        fixed($d100) volatile var red: [256]u8

        # One image, rows of 8 bytes, left pixel in the high nibble; one
        # palette; cell 0 is the image, cell 1 the image mirrored.
        var images: [128]u8
        var palettes: [48]u8
        const CELLS: [4]u16 = [0, 0, $8000, 0]

        export function main(): u8
            var a: u8
            var b: u8
            var h: u8
            var c: u8
            images[0] = $12
            images[7] = $34
            palettes[3] = $56
            vic4_init()
            actors_sheet(u32(addr(images)), u32(addr(palettes)), u32(addr(CELLS)))
            actors_software($50000, 4, 6, 2, $46000, 4)
            actors_hardware($45000, $47000, 1, 24, 46)
            a = actor_new(ACTOR_SOFTWARE)
            b = actor_new(ACTOR_SOFTWARE)
            h = actor_new(ACTOR_HARDWARE)
            c = actor_new(ACTOR_SOFTWARE)
            actor_draw(a, 100, 13, 0, 0)
            actor_draw(b, 40, 16, 1, 0)
            actor_draw(c, 200, 40, 0, 1)
            actor_draw(h, 10, 20, 0, 1)
            actors_frame()
            dma_copy(COLOUR_RAM, $0f00, 6 * 20)
            dma_copy($50000, $0e80, 6 * 20)
            # Frame 2: a moves along its rows, b goes, c moves down a row.
            actor_draw(a, 120, 13, 0, 0)
            actor_hide(b)
            actor_draw(c, 200, 24, 0, 1)
            actors_frame()
            out[0] = sprite_xy[0]
            out[1] = sprite_xy[1]
            out[2] = sprite_enable
            vic4_palette_edit(0)
            out[3] = red[17]
            out[4] = h
            return a + b + c
        end
        ",
        &["vic4", "actors"],
    );
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping actors: Xemu not found (set MENSCH_XMEGA65)");
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    assert_eq!(run.exit_code, 4);
    let memory = &run.memory;
    // Rows are 4 background characters, 2 slots of 2 and a closing slot of
    // 2: 20 bytes.
    let slot = |row: usize, slot: usize| row * 20 + 8 + slot * 4;
    let screen = |at: usize| &memory[0x0e80 + at..0x0e80 + at + 4];
    let colour = |at: usize| &memory[0x0f00 + at..0x0f00 + at + 4];
    // a: x 100, y 13 = row 1 line 5; characters $1180-$1181 ($46000 / 64),
    // palette group 1. Row 1 shifts the top character down 5 and shows
    // lines 5-7, row 2 shifts it up 3, row 3 shows the bottom one's lines 0-4.
    assert_eq!(screen(slot(1, 0)), &[100, 0xb0, 0x80, 0x11]);
    assert_eq!(colour(slot(1, 0)), &[0x98, 0xe0, 0x08, 0x1f]);
    assert_eq!(screen(slot(2, 0)), &[100, 0x60, 0x80, 0x11]);
    assert_eq!(colour(slot(2, 0)), &[0x98, 0xff, 0x08, 0x1f]);
    assert_eq!(screen(slot(3, 0)), &[100, 0x60, 0x81, 0x11]);
    assert_eq!(colour(slot(3, 0)), &[0x98, 0x1f, 0x08, 0x1f]);
    // b: x 40, y 16 = row 2 line 0, slot 1 in rows 2 and 3; its cell is the
    // stored image mirrored, so the characters flip. Same palette, same group.
    assert_eq!(screen(slot(2, 1)), &[40, 0x00, 0x82, 0x11]);
    assert_eq!(colour(slot(2, 1)), &[0x98, 0xff, 0x48, 0x1f]);
    assert_eq!(screen(slot(3, 1)), &[40, 0x00, 0x83, 0x11]);
    // c: cell 0 mirrored, at row 5: a's characters, flipped.
    assert_eq!(screen(slot(5, 0)), &[200, 0x00, 0x80, 0x11]);
    assert_eq!(colour(slot(5, 0)), &[0x98, 0xff, 0x48, 0x1f]);
    // Unused slots point past x 320; each row closes with a token there
    // and a character.
    assert_eq!(&screen(slot(1, 1))[..2], &[0x40, 0x01]);
    assert_eq!(screen(slot(1, 2)), &[0x40, 0x01, 0, 0]);
    assert_eq!(colour(slot(1, 2)), &[0x98, 0xff, 0, 0]);
    // Frame 2. a keeps its slots with the new x; b's slots are off screen;
    // c freed row 5 and took the slot b left in row 3, and one in row 4.
    let now = |at: usize| &memory[0x50000 + at..0x50000 + at + 4];
    assert_eq!(now(slot(1, 0)), &[120, 0xb0, 0x80, 0x11]);
    assert_eq!(now(slot(2, 0)), &[120, 0x60, 0x80, 0x11]);
    assert_eq!(now(slot(3, 0)), &[120, 0x60, 0x81, 0x11]);
    assert_eq!(&now(slot(2, 1))[..2], &[0x40, 0x01]);
    assert_eq!(now(slot(3, 1)), &[200, 0x00, 0x80, 0x11]);
    assert_eq!(now(slot(4, 0)), &[200, 0x00, 0x81, 0x11]);
    assert_eq!(&now(slot(5, 0))[..2], &[0x40, 0x01]);
    // The pool holds each cell's image once, each byte's pixels swapped.
    assert_eq!(&memory[0x46000..0x46008], &[0x21, 0, 0, 0, 0, 0, 0, 0x43]);
    // The hardware actor's buffer holds the image mirrored, and sprite 0
    // sits at (10 + 24, 20 + 46), on.
    assert_eq!(&memory[0x47000..0x47008], &[0x43, 0, 0, 0, 0, 0, 0, 0x21]);
    assert_eq!(&memory[0x0e00..0x0e05], &[34, 66, 0x01, 0x65, 2]);
}

#[test]
fn actors_priority_takes_slots_and_the_loser_retries() {
    let (dir, rom) = library_rom(
        "actors-priority",
        "
        module ActorsPriorityTest
        import Vic4
        import Actors
        fixed($0e00) volatile var out: [8]u8

        var images: [128]u8
        var palettes: [48]u8
        const CELLS: [2]u16 = [0, 0]

        export function main(): u8
            var p: u8
            var q: u8
            var b: u8
            vic4_init()
            actors_sheet(u32(addr(images)), u32(addr(palettes)), u32(addr(CELLS)))
            # Two slots a row: bubbles p and q fill rows 1 and 2.
            actors_software($50000, 4, 6, 2, $46000, 4)
            p = actor_new(ACTOR_SOFTWARE)
            q = actor_new(ACTOR_SOFTWARE)
            b = actor_new(ACTOR_SOFTWARE)
            actor_priority(b, 1)
            actor_draw(p, 30, 8, 0, 0)
            actor_draw(q, 70, 8, 0, 0)
            actor_draw(b, 50, 8, 0, 0)
            actors_frame()
            out[0] = u8(actors_missing())
            dma_copy($50000, $0f00, 6 * 20)
            # Nothing moves; p is still short and tries again.
            actors_frame()
            out[1] = u8(actors_missing())
            # b goes: p is placed before b gives its slots back ...
            actor_hide(b)
            actors_frame()
            out[2] = u8(actors_missing())
            # ... and gets them the frame after.
            actors_frame()
            out[3] = u8(actors_missing())
            return 0
        end
        ",
        &["vic4", "actors"],
    );
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping actors priority: Xemu not found (set MENSCH_XMEGA65)");
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    assert_eq!(run.exit_code, 0);
    let memory = &run.memory;
    let slot = |row: usize, slot: usize| row * 20 + 8 + slot * 4;
    // Frame 1: b took p's slot 0 in both rows; q keeps slot 1. p's two rows
    // are missing, frame after frame, until b goes.
    let first = |at: usize| memory[0x0f00 + at];
    assert_eq!(first(slot(1, 0)), 50);
    assert_eq!(first(slot(2, 0)), 50);
    assert_eq!(first(slot(1, 1)), 70);
    assert_eq!(first(slot(2, 1)), 70);
    assert_eq!(&memory[0x0e00..0x0e04], &[2, 2, 2, 0]);
    let now = |at: usize| memory[0x50000 + at];
    assert_eq!(now(slot(1, 0)), 30);
    assert_eq!(now(slot(2, 0)), 30);
    assert_eq!(now(slot(1, 1)), 70);
}

#[test]
fn bubble_physics_walks_jumps_lands_and_wraps() {
    let physics = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("games/bubble-bobble/src/bubble-physics.mensch"),
    )
    .expect("physics module should be readable");
    let caller = "
        module PhysicsTest
        import BubblePhysics
        fixed($0e00) volatile var out: [23]i16
        var room_map: [832]u8

        # Side walls; ceiling and floor with a gap at columns 14-17; one
        # platform on row 20, columns 8-15.
        procedure build()
            var row: u8
            var col: u8
            var i: u16
            i = 0
            for row = 0 to 25
                for col = 0 to 31
                    room_map[i] = 0
                    if col < 2 or col >= 30
                        room_map[i] = 1
                    elsif (row == 0 or row == 25) and (col < 14 or col > 17)
                        room_map[i] = 5
                    elsif row == 20 and col >= 8 and col <= 15
                        room_map[i] = 5
                    end
                    i = i + 1
                end
            end
            return
        end

        procedure steps(body: u8, frames: u16)
            while frames
                body_step(body)
                frames = frames - 1
            end
            return
        end

        procedure walks(body: u8, dx: i16, frames: u16)
            var stopped: u8
            while frames
                stopped = body_walk(body, dx)
                body_step(body)
                frames = frames - 1
            end
            return
        end

        export function main(): u8
            var frame: u16
            var went_above: i16
            build()
            physics_room(u32(addr(room_map)))
            # 1. A drop lands on the floor (row 25, top at y 200).
            body_place(0, 40, 100)
            steps(0, 200)
            out[0] = body_y(0)
            out[1] = i16(body_on_ground(0))
            # 2. Walking right stops at the wall (column 30, x 240).
            body_place(1, 200, 184)
            walks(1, 16, 60)
            out[2] = body_x(1)
            # 3. A jump from the floor goes up through the platform and
            #    lands on it (row 20, top at y 160).
            body_place(2, 80, 184)
            steps(2, 1)
            out[3] = i16(body_jump(2))
            steps(2, 120)
            out[4] = body_y(2)
            out[5] = i16(body_on_ground(2))
            # 4. Walking left off the platform falls to the floor and on to
            #    the left wall (column 1 ends at x 16).
            walks(2, -16, 160)
            out[6] = body_x(2)
            out[7] = body_y(2)
            # 5. Over the floor gap: out of the bottom, in through the
            #    ceiling gap, onto the platform.
            body_place(3, 116, 184)
            went_above = 0
            for frame = 1 to 600
                body_step(3)
                if body_y(3) < 0
                    went_above = 1
                end
            end
            out[8] = went_above
            out[9] = body_y(3)
            # 6. Overlap, with 3 px margins: 8 px apart touch, 10 do not.
            body_place(4, 50, 50)
            body_place(5, 58, 50)
            out[10] = i16(bodies_touch(4, 5, 3))
            body_place(5, 60, 50)
            out[11] = i16(bodies_touch(4, 5, 3))
            # 7. Falling in mid-air, then launched upward (a bounce).
            body_place(6, 40, 60)
            steps(6, 10)
            out[12] = i16(body_falling(6))
            out[13] = body_y(6)
            body_launch(6)
            steps(6, 4)
            out[14] = i16(body_falling(6))
            out[15] = body_y(6)
            # 8. Launched upward near the top (under the solid ceiling): out
            #    of the top, in at the bottom, up through the floor and back
            #    down onto it.
            body_place(7, 40, 20)
            body_launch(7)
            went_above = 0
            for frame = 1 to 60
                body_step(7)
                if body_y(7) > 150
                    went_above = 1
                end
            end
            out[16] = went_above
            steps(7, 200)
            out[17] = body_y(7)
            out[18] = i16(body_on_ground(7))
            # 9. The arcade's jump on open floor: 60 moves in the air (50
            #    frames at its 6 moves per 5), 42 px up at the top.
            body_place(8, 150, 184)
            steps(8, 1)
            frame = 0
            went_above = 184
            if body_jump(8)
                repeat
                    body_step(8)
                    frame = frame + 1
                    if body_y(8) < went_above
                        went_above = body_y(8)
                    end
                until body_on_ground(8) or frame == 200
            end
            out[19] = i16(frame)
            out[20] = went_above
            # 10. Walking off the platform's left end (40 px above the
            #     floor, as round 1's): 36 frames to land, as the arcade.
            body_place(9, 70, 144)
            steps(9, 1)
            repeat
                frame = u16(body_walk(9, -16))
                body_step(9)
            until body_on_ground(9) == 0
            frame = 0
            repeat
                body_step(9)
                frame = frame + 1
            until body_on_ground(9) or frame == 200
            out[21] = i16(frame)
            out[22] = body_y(9)
            return 0
        end
        ";
    let sources = [
        mega65::KernelModule {
            name: "0-caller".to_owned(),
            source: caller.to_owned(),
            window: None,
        },
        mega65::KernelModule {
            name: "1-physics".to_owned(),
            source: physics,
            window: None,
        },
    ];
    let modules = mega65::compile_kernel_modules(&sources).expect("modules should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir("bubble-physics");
    let rom = mega65::build_rom(&dir, &refs).expect("modules should link");
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping bubble-physics: Xemu not found (set MENSCH_XMEGA65)");
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    assert_eq!(run.exit_code, 0);
    let out: Vec<i16> = (0..23)
        .map(|i| i16::from_le_bytes([run.memory[0x0e00 + 2 * i], run.memory[0x0e01 + 2 * i]]))
        .collect();
    assert_eq!(&out[0..2], &[184, 1], "drop onto the floor");
    assert_eq!(out[2], 224, "walk into the right wall");
    assert_eq!(
        &out[3..6],
        &[1, 144, 1],
        "jump up through the platform onto it"
    );
    assert_eq!(
        &out[6..8],
        &[16, 184],
        "walk off the platform to the floor and the left wall"
    );
    assert_eq!(
        &out[8..10],
        &[1, 144],
        "wrap through the floor and ceiling gaps"
    );
    assert_eq!(
        &out[10..12],
        &[1, 0],
        "overlap inside and outside the margin"
    );
    assert_eq!(out[12], 1, "falling in mid-air");
    assert_eq!(out[14], 0, "no longer falling after a launch");
    assert!(
        out[15] < out[13],
        "launched upward: {} -> {}",
        out[13],
        out[15]
    );
    assert_eq!(out[16], 1, "out of the top, in at the bottom");
    assert_eq!(&out[17..19], &[184, 1], "then down onto the floor");
    assert_eq!(&out[19..21], &[60, 142], "the arcade's jump: 60 moves, 42 px");
    assert_eq!(&out[21..23], &[36, 184], "the arcade's 40 px ledge fall: 36 frames");
}

#[test]
fn bubble_bubbles_shoot_float_trap_and_burst() {
    let read = |name: &str| {
        std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("games/bubble-bobble/src")
                .join(name),
        )
        .expect("module should be readable")
    };
    let caller = "
        module BubblesTest
        import BubblePhysics
        import BubbleBubbles
        import BubbleAir
        fixed($0e00) volatile var out: [16]i16
        var room_map: [832]u8

        # Side walls and a floor.
        procedure build()
            var row: u8
            var col: u8
            var i: u16
            i = 0
            for row = 0 to 25
                for col = 0 to 31
                    room_map[i] = 0
                    if col < 2 or col >= 30
                        room_map[i] = 1
                    elsif row == 25
                        room_map[i] = 5
                    end
                    i = i + 1
                end
            end
            return
        end

        procedure steps(frames: u16)
            while frames
                bubbles_step()
                frames = frames - 1
            end
            return
        end

        export function main(): u8
            var b: u8
            var c: u8
            build()
            physics_room(u32(addr(room_map)))
            bubbles_air(u32(addr(AIR_ROUND1)))
            bubbles_reset()
            # 1. Blown right from x 40 (as the arcade's): shows at 42, then
            #    shoots 3 px a frame for 21 frames and floats.
            b = bubble_blow(40, 184, 0, 8)
            out[0] = i16(b)
            steps(1 + SHOT_FRAMES)
            out[1] = bubble_x(b)
            out[2] = i16(bubble_state(b))
            # 2. Rises on round 1's air (its column, 14, goes up to the top
            #    rows, which go down) and bobs between y 16 and 24.
            steps(450)
            out[3] = bubble_y(b)
            out[4] = bubble_x(b)
            # 3. Bursts when its life of 8 runs out (8 * 60 + 23 floating
            #    frames), then is free.
            steps(53)
            out[5] = i16(bubble_state(b))
            steps(POP_FRAMES)
            out[6] = i16(bubble_state(b))
            # 4. Blown at the right wall (column 30, x 240): stops at 224.
            b = bubble_blow(200, 184, 0, 8)
            steps(9)
            out[7] = bubble_x(b)
            out[8] = i16(bubble_state(b))
            # 5. A trapped enemy floats, then breaks out after round 1's 1800
            #    frames.
            bubbles_reset()
            b = bubble_blow(40, 184, 0, 8)
            out[9] = i16(bubble_capture(b, 2))
            out[10] = i16(bubble_state(b))
            steps(1799)
            out[11] = i16(bubble_escapee(b))
            steps(1)
            out[12] = i16(bubble_escapee(b))
            # 6. Popping an empty floating bubble frees no one.
            c = bubble_blow(80, 184, 1, 8)
            steps(SHOT_FRAMES)
            out[13] = i16(bubble_pop(c))
            out[14] = i16(bubble_state(c))
            return 0
        end
        ";
    let sources = [
        ("0-caller", caller.to_owned()),
        ("1-physics", read("bubble-physics.mensch")),
        ("2-bubbles", read("bubble-bubbles.mensch")),
        ("3-air", read("bubble-air.mensch")),
    ]
    .map(|(name, source)| mega65::KernelModule {
        name: name.to_owned(),
        source,
        window: None,
    });
    let modules = mega65::compile_kernel_modules(&sources).expect("modules should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir("bubble-bubbles");
    let rom = mega65::build_rom(&dir, &refs).expect("modules should link");
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping bubble-bubbles: Xemu not found (set MENSCH_XMEGA65)");
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    assert_eq!(run.exit_code, 0);
    let out: Vec<i16> = (0..15)
        .map(|i| i16::from_le_bytes([run.memory[0x0e00 + 2 * i], run.memory[0x0e01 + 2 * i]]))
        .collect();
    // States: free 0, shot 1, float 2, pop 3; no captive is $FF (-1 as i16 of u8 is 255).
    assert_eq!(&out[0..3], &[0, 105, 2], "shoots 63 px from x 42, then floats");
    assert!(
        (16..=24).contains(&out[3]) && out[4] == 106,
        "bobs at the top of round 1's air: ({}, {})",
        out[4],
        out[3]
    );
    assert_eq!(&out[5..7], &[3, 0], "bursts, then is free");
    assert_eq!(&out[7..9], &[224, 2], "stops at the wall and floats");
    assert_eq!(&out[9..11], &[1, 2], "traps an enemy and floats");
    assert_eq!(
        &out[11..13],
        &[255, 2],
        "the enemy breaks out at 1800 frames"
    );
    assert_eq!(
        &out[13..15],
        &[255, 3],
        "an empty bubble pops with no captive"
    );
}

/// Names ca65 would read as registers or instructions (`x`, `row`, `bit`)
/// work as ordinary names, across modules too.
#[test]
fn assembler_names_work_across_modules() {
    let library = "
        module Clashes
        export var x: u8
        var row: u16
        export function bit(): u8
            row = u16(x) * 3
            return u8(row) + 1
        end
    ";
    let caller = "
        module ClashTest
        import Clashes
        fixed($0e00) volatile var out: [2]u8
        export function main(): u8
            x = 13
            out[0] = bit()
            out[1] = x
            return 0
        end
    ";
    let sources = [
        mega65::KernelModule {
            name: "0-caller".to_owned(),
            source: caller.to_owned(),
            window: None,
        },
        mega65::KernelModule {
            name: "1-clashes".to_owned(),
            source: library.to_owned(),
            window: None,
        },
    ];
    let modules = mega65::compile_kernel_modules(&sources).expect("modules should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir("assembler-names");
    let rom = mega65::build_rom(&dir, &refs).expect("modules should link");
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping assembler-names: Xemu not found (set MENSCH_XMEGA65)");
        return;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    assert_eq!(run.exit_code, 0);
    assert_eq!(&run.memory[0x0e00..0x0e02], &[40, 13]);
}

// ---- benchmarks ----------------------------------------------------------
//
// Compiled code timed in Xemu: `body` runs between two readings of the
// machine's frame counter and raster, and the time in raster lines (64 µs
// each) is printed. Each benchmark fails past its limit (about 10% over
// what it takes now), so a slower code generator is noticed.

/// Builds and runs a benchmark module (its `imports`, declarations,
/// `setup` and timed `body`) with `extra` modules; the raster lines taken,
/// or `None` without Xemu.
fn bench_lines(
    name: &str,
    imports: &str,
    decls: &str,
    setup: &str,
    body: &str,
    extra: Vec<(&str, String)>,
) -> Option<u32> {
    let caller = format!(
        "
module Bench
{imports}
fixed($d7fa) volatile var frame_counter: u8
fixed($d012) volatile var raster_low: u8
fixed($d011) volatile var raster_high: u8
fixed($0e00) volatile var out: [2]u16
var bench_frames: u8
var bench_line: u16

# The frame counter and raster line, read until they agree.
procedure bench_now()
    repeat
        bench_frames = frame_counter
        bench_line = u16(raster_low)
        if raster_high & $80
            bench_line = bench_line + 256
        end
    until bench_frames == frame_counter
    return
end

{decls}

export function main(): u8
    var first_frames: u8
    var first_line: u16
{setup}
    bench_now()
    first_frames = bench_frames
    first_line = bench_line
{body}
    bench_now()
    out[0] = u16(bench_frames - first_frames) * 312 + bench_line - first_line
    return 0
end
"
    );
    let mut sources = vec![mega65::KernelModule {
        name: "0-bench".to_owned(),
        source: caller,
        window: None,
    }];
    for (index, (module, source)) in extra.into_iter().enumerate() {
        sources.push(mega65::KernelModule {
            name: format!("{}-{module}", index + 1),
            source,
            window: None,
        });
    }
    let modules = mega65::compile_kernel_modules(&sources).expect("benchmark should compile");
    let refs = modules
        .iter()
        .map(|(name, asm)| (name.as_str(), asm.as_str()))
        .collect::<Vec<_>>();
    let dir = temp_dir(name);
    let rom = mega65::build_rom(&dir, &refs).expect("benchmark should link");
    let Some(xemu) = mega65::find_xemu() else {
        eprintln!("skipping {name}: Xemu not found (set MENSCH_XMEGA65)");
        return None;
    };
    let run = mega65::run_rom(&xemu, &dir, &rom, Duration::from_secs(30)).unwrap();
    assert_eq!(run.exit_code, 0);
    let lines = u32::from(u16::from_le_bytes([run.memory[0x0e00], run.memory[0x0e01]]));
    eprintln!("{name}: {lines} raster lines");
    Some(lines)
}

fn repo_file(path: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path))
        .expect("source should be readable")
}

fn assert_within(name: &str, lines: Option<u32>, limit: u32) {
    if let Some(lines) = lines {
        assert!(lines > 0 && lines <= limit, "{name} took {lines} raster lines (limit {limit})");
    }
}

/// A rotozoomer-style column loop in plain Mensch: a texel read through a
/// far pointer, two 16-bit writes through another, 16- and 32-bit steps;
/// 10,000 iterations.
#[test]
fn texel_loop_benchmark() {
    let decls = "
var start_at: u32
var u0: u16
var v0: u16
var du: u16
var dv: u16

procedure column()
    var at: u32
    var u: u16
    var v: u16
    var cell: far ^u16
    var texel: u16
    var texture: far ^u8
    var block_y: u8
    at = start_at
    u = u0
    v = v0
    block_y = 0
    while block_y != 100
        texture = $10000 + u32(((v >> 8) & $7f) << 8) + u32(u >> 8)
        texel = u16(texture^)
        texel = texel | (texel << 8)
        cell = at
        cell^ = texel
        cell = at + 8
        cell^ = texel
        at = at + 16
        u = u + du
        v = v + dv
        block_y = block_y + 1
    end
    return
end
";
    let setup = "
    var c: u8
    du = 300
    dv = 120
";
    let body = "
    c = 0
    while c != 100
        start_at = $40000 + u32(u16(c >> 3) * 1600) + u32((c << 1) & 7)
        u0 = u16(c) * 77
        v0 = u16(c) * 31
        column()
        c = c + 1
    end
";
    let lines = bench_lines("texel-loop-benchmark", "", decls, setup, body, vec![]);
    assert_within("texel loop", lines, 1170);
}

/// The demo's dots past the glass, in the same style: module variables,
/// table lookups, branches, a pixel address from a table of column offsets
/// and two plots a dot; 64 columns of 50 samples, four times over.
#[test]
fn dot_loop_benchmark() {
    let decls = "
const SINE: [256]i8 = [SINE_TABLE]
const LEVEL: [64]u8 = [LEVEL_TABLE]
const COLUMN_OFFSET: [40]u16 = [OFFSET_TABLE]
const DOTS = 64
const RUNNER_Y = 70

var t: u16
var sx: i16
var ry: i16
var y_end: i16
var past: i16
var push: i16
var dy: i16
var fade: i16
var thin: i16
var wave: u8
var hash: u16
var level: i16
var px: i16
var py: i16
var pixel: u8
var source: far ^u8
var dot: far ^u8

function chunky_at(x: u16, y: u16): u32
    return $40000 + u32(COLUMN_OFFSET[u8(x >> 3)] + (y << 3) + (x & 7))
end

procedure dots()
    past = sx - 100
    push = sx + ((past * past) >> 5)
    dy = (i16(SINE[u8(u16(sx) * 2 + t * 3)]) * past) >> 9
    fade = past >> 3
    thin = past >> 2
    wave = u8(t << 2) + u8(RUNNER_Y * 3)
    hash = u16(sx) * 7 + u16(RUNNER_Y) * 13 + t
    source = $10000 + u32(u16(sx) * 100)
    ry = RUNNER_Y
    y_end = RUNNER_Y + 100
    while ry < y_end
        pixel = source^
        if pixel
            level = i16(LEVEL[pixel & 63]) - fade
            if level > 0 and i16(hash & 31) >= thin
                px = push + i16(SINE[wave] >> 3)
                py = ry + dy
                if px >= 1 and px < 320 and py >= 0 and py < 200
                    dot = chunky_at(u16(px), u16(py))
                    dot^ = DOTS + u8(level)
                    if level > 1
                        if (px & 7) == 0
                            dot = dot - 1593
                        else
                            dot = dot - 1
                        end
                        dot^ = DOTS + u8(level >> 1)
                    end
                end
            end
        end
        source = source + 2
        wave = wave + 6
        hash = hash + 26
        ry = ry + 2
    end
    return
end
"
    .replace("SINE_TABLE", "0, 3, 6, 9, 12, 16, 19, 22, 25, 28, 31, 34, 37, 40, 43, 46, 49, 51, 54, 57, 60, 63, 65, 68, 71, 73, 76, 78, 81, 83, 85, 88, 90, 92, 94, 96, 98, 100, 102, 104, 106, 107, 109, 111, 112, 113, 115, 116, 117, 118, 120, 121, 122, 122, 123, 124, 125, 125, 126, 126, 126, 127, 127, 127, 127, 127, 127, 127, 126, 126, 126, 125, 125, 124, 123, 122, 122, 121, 120, 118, 117, 116, 115, 113, 112, 111, 109, 107, 106, 104, 102, 100, 98, 96, 94, 92, 90, 88, 85, 83, 81, 78, 76, 73, 71, 68, 65, 63, 60, 57, 54, 51, 49, 46, 43, 40, 37, 34, 31, 28, 25, 22, 19, 16, 12, 9, 6, 3, 0, -3, -6, -9, -12, -16, -19, -22, -25, -28, -31, -34, -37, -40, -43, -46, -49, -51, -54, -57, -60, -63, -65, -68, -71, -73, -76, -78, -81, -83, -85, -88, -90, -92, -94, -96, -98, -100, -102, -104, -106, -107, -109, -111, -112, -113, -115, -116, -117, -118, -120, -121, -122, -122, -123, -124, -125, -125, -126, -126, -126, -127, -127, -127, -127, -127, -127, -127, -126, -126, -126, -125, -125, -124, -123, -122, -122, -121, -120, -118, -117, -116, -115, -113, -112, -111, -109, -107, -106, -104, -102, -100, -98, -96, -94, -92, -90, -88, -85, -83, -81, -78, -76, -73, -71, -68, -65, -63, -60, -57, -54, -51, -49, -46, -43, -40, -37, -34, -31, -28, -25, -22, -19, -16, -12, -9, -6, -3")
    .replace("LEVEL_TABLE", "0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15")
    .replace("OFFSET_TABLE", "0, 1600, 3200, 4800, 6400, 8000, 9600, 11200, 12800, 14400, 16000, 17600, 19200, 20800, 22400, 24000, 25600, 27200, 28800, 30400, 32000, 33600, 35200, 36800, 38400, 40000, 41600, 43200, 44800, 46400, 48000, 49600, 51200, 52800, 54400, 56000, 57600, 59200, 60800, 62400");
    let setup = "
    var fill: far ^u8
    var i: u16
    var c: u8
    var pass: u8
    # Something to draw: a pattern of palette entries, zero now and then.
    fill = $10000
    i = 0
    while i != 12800
        fill^ = u8(i * 7) & 63
        fill = fill + 1
        i = i + 1
    end
";
    let body = "
    pass = 0
    while pass != 4
        t = u16(pass) * 5
        c = 0
        while c != 64
            sx = 110 + i16(c)
            dots()
            c = c + 1
        end
        pass = pass + 1
    end
";
    let lines = bench_lines("dot-loop-benchmark", "", &decls, setup, body, vec![]);
    assert_within("dot loop", lines, 3680);
}

/// DMA set-up: 2,000 small fills through lib/vic4, so the time is mostly
/// building the job.
#[test]
fn dma_setup_benchmark() {
    let setup = "
    var i: u16
";
    let body = "
    i = 0
    while i != 2000
        dma_fill($40000 + u32(i << 3), u8(i), 8)
        i = i + 1
    end
";
    let lines = bench_lines(
        "dma-setup-benchmark",
        "import Vic4",
        "",
        setup,
        body,
        vec![("vic4", repo_file("lib/vic4.mensch"))],
    );
    assert_within("dma setup", lines, 560);
}

/// Game code: the Bubble Bobble port's physics stepping and walking ten
/// bodies around a room for 100 frames.
#[test]
fn physics_benchmark() {
    let decls = "
var room_map: [832]u8

procedure build()
    var row: u8
    var col: u8
    var i: u16
    i = 0
    for row = 0 to 25
        for col = 0 to 31
            room_map[i] = 0
            if col < 2 or col >= 30
                room_map[i] = 1
            elsif row == 25 or (row == 20 and col >= 8 and col <= 15)
                room_map[i] = 5
            end
            i = i + 1
        end
    end
    return
end
";
    let setup = "
    var body: u8
    var frame: u8
    var stopped: u8
    build()
    physics_room(u32(addr(room_map)))
    body = 0
    while body != 10
        body_place(body, 30 + i16(body) * 20, 40)
        body = body + 1
    end
";
    let body = "
    frame = 0
    while frame != 100
        body = 0
        while body != 10
            if body & 1
                stopped = body_walk(body, 16)
            else
                stopped = body_walk(body, -16)
            end
            body_step(body)
            body = body + 1
        end
        frame = frame + 1
    end
";
    let lines = bench_lines(
        "physics-benchmark",
        "import BubblePhysics",
        decls,
        setup,
        body,
        vec![("physics", repo_file("games/bubble-bobble/src/bubble-physics.mensch"))],
    );
    assert_within("physics", lines, 1880);
}
