//! MEGA65 bare-metal runtime, ROM image builder, and Xemu test runner.
//!
//! A Mensch program for the MEGA65 is linked into a 128 KiB image that
//! replaces the C65 ROM (`MEGA65.ROM` on a real machine, `-rom` in Xemu).
//! The hypervisor loads that file at linear `$20000` and, in C64 memory
//! mode, shows its top 8 KiB at `$E000-$FFFF` before jumping through the
//! reset vector.
//!
//! The kernel is linked at bank-0 addresses equal to its offsets in the ROM
//! file, so loading is a straight copy:
//!
//! ```text
//! $0002-$00ff   base page (runtime registers, direct-page variables)
//! $0200-$1fff   free low RAM (screen, buffers, test output)
//! $2000-$cfff   kernel image: code, data, then BSS; 16-bit stack grows
//!               down from $cfff towards the end of BSS
//! $d000-$dfff   I/O
//! $e000-$feff   stage-1 loader (ROM window), copied to RAM by stage 2
//! $ff00-$ff1f   trap vector table
//! $fffa-$ffff   NMI / RESET / IRQ vectors
//! ```
//!
//! Stage 1 (in the ROM window) maps BASIC out and copies `$2000-$cfff` from
//! the ROM file. Stage 2 (in RAM) maps the KERNAL ROM out, copies the
//! `$e000` window into the RAM underneath, clears base page and BSS, and
//! calls `main`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const ROM_SIZE: usize = 0x20000;
/// Size of the main-RAM dump Xemu writes with `-dumpmem`.
pub const MAIN_RAM_SIZE: usize = 0x60000;
/// Linear address where the hypervisor loads the ROM file.
pub const ROM_LOAD_ADDRESS: u32 = 0x20000;

/// Startup code: the two-stage loader, base-page and BSS clearing,
/// `jsr main`, then exit through Xemu's testing register with `main`'s `A`
/// as the status.
pub const CRT0: &str = r#".setcpu "45GS02"
.exportzp __m65_acc, __m65_ptr, __m65_farp, __m65_fp
.export __m65_enter, __m65_leave
.import main
.import __BSS_RUN__, __BSS_SIZE__

__m65_stack_top = $cfff
__m65_image_start = $2000
__m65_image_end = $d000
__m65_rom_linear = $020000

.segment "ZEROPAGE"
__m65_acc: .res 4
__m65_ptr: .res 2
__m65_farp: .res 4
__m65_fp: .res 2

; Copy `count` bytes from linear `source` to bank-0 `dest`. Neither range
; may cross a 64 KiB boundary (only the low word of the source advances).
.macro copy_linear source, dest, count
	.local loop, done
	lda #<(source)
	sta __m65_farp
	lda #>(source)
	sta __m65_farp+1
	lda #^(source)
	sta __m65_farp+2
	stz __m65_farp+3
	lda #<(dest)
	sta __m65_ptr
	lda #>(dest)
	sta __m65_ptr+1
	lda #<(count)
	sta __m65_acc
	lda #>(count)
	sta __m65_acc+1
loop:
	lda __m65_acc
	ora __m65_acc+1
	beq done
	lda [__m65_farp],z
	sta (__m65_ptr),z
	inw __m65_farp
	inw __m65_ptr
	dew __m65_acc
	bra loop
done:
.endmacro

.segment "STARTUP"
__m65_reset:
	sei
	cld
	cle                 ; 16-bit stack pointer
	ldx #<__m65_stack_top
	txs
	ldy #>__m65_stack_top
	tys
	ldz #$00            ; codegen invariant: Z = 0
	lda #$00            ; base page is page 0
	tab
	lda #$47            ; select the MEGA65 (VIC-IV) I/O personality
	sta $d02f
	lda #$53
	sta $d02f
	lda #$40            ; 40.5 MHz (VIC-III FAST + VIC-IV VFAST)
	tsb $d031
	tsb $d054
	lda #$f8            ; no C65 ROM banking at $8000-$ffff
	trb $d030
	lda #$2f            ; standard CPU port direction bits
	sta $00
	lda #$36            ; BASIC out, KERNAL (this window) and I/O in
	sta $01
	copy_linear __m65_rom_linear + __m65_image_start, __m65_image_start, __m65_image_end - __m65_image_start
	jmp __m65_stage2

.segment "KERNEL_START"
__m65_stage2:
	lda #$35            ; all RAM except I/O
	sta $01
	copy_linear __m65_rom_linear + $e000, $e000, $2000
	ldx #$02            ; clear base page (keep the CPU port at $00/$01)
@clear_bp:
	stz $00,x
	inx
	bne @clear_bp
	lda #<__BSS_RUN__
	sta __m65_ptr
	lda #>__BSS_RUN__
	sta __m65_ptr+1
	lda #<__BSS_SIZE__
	sta __m65_acc
	lda #>__BSS_SIZE__
	sta __m65_acc+1
@clear_bss:
	lda __m65_acc
	ora __m65_acc+1
	beq @bss_done
	lda #$00
	sta (__m65_ptr),z
	inw __m65_ptr
	dew __m65_acc
	bra @clear_bss
@bss_done:
	jsr main
	sta $d6cf           ; exit status (Xemu -testing)
	lda #$42
	sta $d6cf           ; request exit
@hang:
	bra @hang

.segment "CODE"
; Stack frames (see lower_45gs02). A routine with a frame starts with
; `ldz #F / jsr __m65_enter` (A, X and Y are free there): this pushes the
; caller's fp and F zeroed local bytes beneath the routine's return address,
; points fp at the new frame and returns with Z = 0.
__m65_enter:
	pla                 ; our return address (the JSR's last byte)
	sta __m65_ptr
	pla
	sta __m65_ptr+1
	lda __m65_fp+1
	pha
	lda __m65_fp
	pha
	tza
	tax
	beq @framed
	lda #$00
@zero:
	pha
	dex
	bne @zero
@framed:
	tsx
	stx __m65_fp
	tsy
	sty __m65_fp+1
	ldz #$00
	inw __m65_ptr
	jmp (__m65_ptr)

; The matching epilogue: `ldz #F / jsr __m65_leave`, with the routine's result
; in A/X/Y. Drops the locals and restores the caller's fp, returning with
; A/X/Y intact and Z = 0; the routine then returns with RTS, RTN or RTI.
__m65_leave:
	sta __m65_acc
	stx __m65_acc+1
	sty __m65_acc+2
	pla
	sta __m65_ptr
	pla
	sta __m65_ptr+1
	tza
	tax
	beq @dropped
@drop:
	pla
	dex
	bne @drop
@dropped:
	pla
	sta __m65_fp
	pla
	sta __m65_fp+1
	inw __m65_ptr
	lda __m65_acc
	ldx __m65_acc+1
	ldy __m65_acc+2
	ldz #$00
	jmp (__m65_ptr)

.segment "VECTOR_RESET"
	.addr __m65_reset
"#;

/// ld65 configuration for a 128 KiB MEGA65 ROM image (see the module docs).
pub const LINKER_CONFIG: &str = r#"MEMORY {
  ZP:    start = $0002,  size = $00fe, type = rw;
  LOW:   start = $0000,  size = $2000, file = %O, fill = yes;
  IMAGE: start = $2000,  size = $b000, file = %O, fill = yes;
  IO:    start = $d000,  size = $1000, file = %O, fill = yes;
  ROM:   start = $e000,  size = $1f00, file = %O, fill = yes;
  TRAP:  start = $ff00,  size = $0020, file = %O, fill = yes;
  TAIL:  start = $ff20,  size = $00da, file = %O, fill = yes;
  VEC:   start = $fffa,  size = $0006, file = %O, fill = yes;
  HIGH:  start = $10000, size = $10000, file = %O, fill = yes;
}
SEGMENTS {
  ZEROPAGE:     load = ZP,    type = zp;
  KERNEL_START: load = IMAGE, type = ro;
  LOWCODE:      load = IMAGE, type = ro, optional = yes;
  CODE:         load = IMAGE, type = ro;
  RODATA:       load = IMAGE, type = ro, optional = yes;
  MENSCH_META:  load = IMAGE, type = ro, optional = yes;
  BANK1:        load = IMAGE, type = ro, optional = yes;
  BSS:          load = IMAGE, type = bss, define = yes;
  STARTUP:      load = ROM,   type = ro;
  VECTOR_TRAP_00: load = TRAP, type = ro, optional = yes, offset = $0000;
  VECTOR_TRAP_01: load = TRAP, type = ro, optional = yes, offset = $0002;
  VECTOR_TRAP_02: load = TRAP, type = ro, optional = yes, offset = $0004;
  VECTOR_TRAP_03: load = TRAP, type = ro, optional = yes, offset = $0006;
  VECTOR_TRAP_04: load = TRAP, type = ro, optional = yes, offset = $0008;
  VECTOR_TRAP_05: load = TRAP, type = ro, optional = yes, offset = $000a;
  VECTOR_TRAP_06: load = TRAP, type = ro, optional = yes, offset = $000c;
  VECTOR_TRAP_07: load = TRAP, type = ro, optional = yes, offset = $000e;
  VECTOR_TRAP_08: load = TRAP, type = ro, optional = yes, offset = $0010;
  VECTOR_TRAP_09: load = TRAP, type = ro, optional = yes, offset = $0012;
  VECTOR_TRAP_0A: load = TRAP, type = ro, optional = yes, offset = $0014;
  VECTOR_TRAP_0B: load = TRAP, type = ro, optional = yes, offset = $0016;
  VECTOR_TRAP_0C: load = TRAP, type = ro, optional = yes, offset = $0018;
  VECTOR_TRAP_0D: load = TRAP, type = ro, optional = yes, offset = $001a;
  VECTOR_TRAP_0E: load = TRAP, type = ro, optional = yes, offset = $001c;
  VECTOR_TRAP_0F: load = TRAP, type = ro, optional = yes, offset = $001e;
  VECTOR_NMI:   load = VEC, type = ro, optional = yes, offset = $0000;
  VECTOR_RESET: load = VEC, type = ro, offset = $0002;
  VECTOR_IRQ:   load = VEC, type = ro, optional = yes, offset = $0004;
}
"#;

/// Base-page bytes available to Mensch code: `$02-$FF` minus the runtime
/// registers defined in [`CRT0`].
pub const BASE_PAGE_BUDGET: u16 = 0xfe - 12;

/// Compiles Mensch modules for the MEGA65 with one shared base-page budget.
/// Returns `(module file stem, assembly)` pairs ready for [`build_rom`].
pub fn compile_modules(sources: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut assignments = Vec::new();
    let mut out = Vec::new();
    for (name, source) in sources {
        let used: u16 = assignments
            .iter()
            .map(|assignment: &crate::storage::StorageAssignment| assignment.direct_page_bytes)
            .sum();
        let compiled = crate::driver::compile_source(
            source,
            crate::driver::Target::Mega65,
            BASE_PAGE_BUDGET.saturating_sub(used),
        )
        .map_err(|error| format!("{name}: {error}"))?;
        assignments.push(compiled.assignment);
        out.push((name.clone(), compiled.asm));
    }
    crate::storage::validate_direct_page_budget(&assignments, BASE_PAGE_BUDGET)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// Assembles and links generated modules with the runtime into a ROM image.
///
/// `modules` are `(name, ca65 source)` pairs. Intermediate files go in `dir`.
pub fn build_rom(dir: &Path, modules: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    fs::create_dir_all(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let cfg = dir.join("mega65-rom.cfg");
    write(&cfg, LINKER_CONFIG)?;

    let mut objects = Vec::new();
    let sources = std::iter::once(("crt0", CRT0)).chain(modules.iter().copied());
    for (name, source) in sources {
        let asm = dir.join(format!("{name}.s"));
        let obj = dir.join(format!("{name}.o"));
        write(&asm, source)?;
        run(Command::new("ca65")
            .arg("--cpu")
            .arg("45GS02")
            .arg("-g")
            .arg(&asm)
            .arg("-o")
            .arg(&obj))?;
        objects.push(obj);
    }

    let rom = dir.join("kernel.rom");
    run(Command::new("ld65")
        .arg("-C")
        .arg(&cfg)
        .arg("-m")
        .arg(dir.join("kernel.map"))
        .arg("-o")
        .arg(&rom)
        .args(&objects))?;
    let image = fs::read(&rom).map_err(|error| format!("read {}: {error}", rom.display()))?;
    if image.len() != ROM_SIZE {
        return Err(format!(
            "linked ROM is {} bytes, expected {ROM_SIZE}",
            image.len()
        ));
    }
    Ok(image)
}

/// Locates the Xemu MEGA65 binary: `$MENSCH_XMEGA65`, then
/// `~/src/xemu/build/bin/xmega65.native`, then `xmega65` on `PATH`.
pub fn find_xemu() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MENSCH_XMEGA65") {
        return Some(PathBuf::from(path));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let path = Path::new(&home).join("src/xemu/build/bin/xmega65.native");
        if path.is_file() {
            return Some(path);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("xmega65"))
            .find(|path| path.is_file())
    })
}

/// Whether this Xemu build has the `-typein` keyboard injection option
/// (a local patch; see `XemuOptions::type_in`).
pub fn xemu_supports_type_in(xemu: &Path) -> bool {
    Command::new(xemu)
        .arg("-h")
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout).contains("-typein")
                || String::from_utf8_lossy(&output.stderr).contains("-typein")
        })
        .unwrap_or(false)
}

#[derive(Debug)]
pub struct XemuRun {
    /// Process exit status: the value the program wrote to `$D6CF` before
    /// exiting, or -1 when the run was stopped at the timeout.
    pub exit_code: i32,
    /// Whether Xemu was stopped at the timeout (see `XemuOptions::stop_at_timeout`).
    pub timed_out: bool,
    /// Main RAM at exit (384 KiB, flat).
    pub memory: Vec<u8>,
    /// The text screen at exit, as Xemu's `-dumpscreen` renders it.
    pub screen: String,
    /// Combined emulator log output, for diagnostics.
    pub log: String,
}

#[derive(Debug, Clone)]
pub struct XemuOptions {
    pub timeout: Duration,
    /// Text to type into the hardware keyboard queue (`$D610`) as the
    /// program reads it; `\n` or `\r` is RETURN. Needs an Xemu with the
    /// `-typein` option.
    pub type_in: Option<String>,
    /// At the timeout, stop Xemu with SIGTERM (it still writes its memory and
    /// screen dumps) and return the run instead of an error. For kernels
    /// that never exit by themselves.
    pub stop_at_timeout: bool,
    /// Files placed in the root directory of the virtual SD card's FAT32
    /// partition, as `(8.3 name, contents)`.
    pub sd_files: Vec<(String, Vec<u8>)>,
}

impl Default for XemuOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            type_in: None,
            stop_at_timeout: false,
            sd_files: Vec::new(),
        }
    }
}

/// Xemu shares preference files between instances, so runs are serialised.
static XEMU_LOCK: Mutex<()> = Mutex::new(());

/// Boots `rom` headless in Xemu and waits for it to exit through `$D6CF`.
pub fn run_rom(xemu: &Path, dir: &Path, rom: &[u8], timeout: Duration) -> Result<XemuRun, String> {
    run_rom_with(
        xemu,
        dir,
        rom,
        &XemuOptions {
            timeout,
            ..XemuOptions::default()
        },
    )
}

/// Boots `rom` headless in Xemu with `options` and waits for it to exit.
pub fn run_rom_with(
    xemu: &Path,
    dir: &Path,
    rom: &[u8],
    options: &XemuOptions,
) -> Result<XemuRun, String> {
    let timeout = options.timeout;
    let _guard = XEMU_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    fs::create_dir_all(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let rom_path = dir.join("kernel.rom");
    let mem_path = dir.join("mem.bin");
    let screen_path = dir.join("screen.txt");
    let _ = fs::remove_file(&screen_path);
    let sd_dir = dir.join("sd");
    let _ = fs::remove_dir_all(&sd_dir);
    fs::create_dir_all(&sd_dir).map_err(|error| format!("create {}: {error}", sd_dir.display()))?;
    for (name, contents) in &options.sd_files {
        let path = sd_dir.join(name);
        fs::write(&path, contents).map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    let _ = fs::remove_file(&mem_path);
    fs::write(&rom_path, rom).map_err(|error| format!("write {}: {error}", rom_path.display()))?;

    let log_path = dir.join("xemu.log");
    let log_file = fs::File::create(&log_path)
        .map_err(|error| format!("create {}: {error}", log_path.display()))?;
    let mut child = Command::new(xemu)
        .args([
            "-headless",
            "-sleepless",
            "-testing",
            "-besure",
            "-nosound",
            "-virtsd",
        ])
        .arg("-sdimg")
        .arg(&sd_dir)
        .arg("-rom")
        .arg(&rom_path)
        .arg("-dumpmem")
        .arg(&mem_path)
        .arg("-dumpscreen")
        .arg(&screen_path)
        .args(
            options
                .type_in
                .iter()
                .flat_map(|text| ["-typein".to_owned(), text.clone()]),
        )
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|error| error.to_string())?)
        .stderr(log_file)
        .spawn()
        .map_err(|error| format!("failed to start {}: {error}", xemu.display()))?;

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline && options.stop_at_timeout && !timed_out {
            // SIGTERM lets Xemu run its exit handlers, which write the dumps.
            timed_out = true;
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(child.id().to_string())
                .status();
            continue;
        }
        if Instant::now() >= deadline + Duration::from_secs(5)
            || (Instant::now() >= deadline && !options.stop_at_timeout)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Xemu did not exit within {timeout:?}; log: {}",
                read_log(&log_path)
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let log = read_log(&log_path);
    let exit_code = if timed_out {
        -1
    } else {
        status
            .code()
            .ok_or_else(|| format!("Xemu was terminated by a signal; log: {log}"))?
    };
    let memory = fs::read(&mem_path)
        .map_err(|error| format!("Xemu wrote no memory dump ({error}); log: {log}"))?;
    let screen = fs::read_to_string(&screen_path).unwrap_or_default();
    Ok(XemuRun {
        exit_code,
        timed_out,
        memory,
        screen,
        log,
    })
}

fn read_log(path: &Path) -> String {
    let mut text = String::new();
    if let Ok(mut file) = fs::File::open(path) {
        let _ = file.read_to_string(&mut text);
    }
    text
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|error| format!("write {}: {error}", path.display()))
}

fn run(command: &mut Command) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("failed to run {command:?}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{command:?} failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
