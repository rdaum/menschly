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
