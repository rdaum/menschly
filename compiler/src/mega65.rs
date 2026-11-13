//! MEGA65 bare-metal runtime, ROM image builder, and Xemu test runner.
//!
//! A Mensch program for the MEGA65 is linked into a 128 KiB image that
//! replaces the C65 ROM (`MEGA65.ROM` on a real machine, `-rom` in Xemu).
//! After the hypervisor finishes booting it maps the ROM's top 8 KiB at
//! `$E000-$FFFF` and jumps through the reset vector, so the kernel's code
//! currently has to fit in that window.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const ROM_SIZE: usize = 0x20000;
/// Size of the main-RAM dump Xemu writes with `-dumpmem`.
pub const MAIN_RAM_SIZE: usize = 0x60000;

/// Startup code: CPU setup, base-page and BSS clearing, `jsr main`, then exit
/// through Xemu's testing register with `main`'s `A` as the status.
pub const CRT0: &str = r#".setcpu "45GS02"
.exportzp __m65_acc, __m65_ptr, __m65_farp, __m65_fp
.import main
.import __BSS_RUN__, __BSS_SIZE__

__m65_stack_top = $7fff

.segment "ZEROPAGE"
__m65_acc: .res 4
__m65_ptr: .res 2
__m65_farp: .res 4
__m65_fp: .res 2

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
	lda #$47            ; select the MEGA65 (VIC-IV) I/O personality
	sta $d02f
	lda #$53
	sta $d02f
	lda #65             ; 40.5 MHz
	sta $00
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

.segment "VECTOR_RESET"
	.addr __m65_reset
"#;

/// ld65 configuration for a 128 KiB MEGA65 ROM image.
pub const LINKER_CONFIG: &str = r#"MEMORY {
  ZP:   start = $0002,  size = $00fe, type = rw;
  PAD:  start = $0000,  size = $e000, file = %O, fill = yes;
  ROM:  start = $e000,  size = $1ffa, file = %O, fill = yes;
  VEC:  start = $fffa,  size = $0006, file = %O, fill = yes;
  HIGH: start = $10000, size = $10000, file = %O, fill = yes;
  BSS:  start = $1000,  size = $6000, type = rw;
  # $7000-$7fff: 16-bit stack, growing down from $7fff
}
SEGMENTS {
  ZEROPAGE:     load = ZP,  type = zp;
  STARTUP:      load = ROM, type = ro;
  CODE:         load = ROM, type = ro;
  RODATA:       load = ROM, type = ro, optional = yes;
  MENSCH_META:  load = ROM, type = ro, optional = yes;
  BSS:          load = BSS, type = bss, define = yes;
  VECTOR_NMI:   load = VEC, type = ro, optional = yes, offset = $0000;
  VECTOR_RESET: load = VEC, type = ro, offset = $0002;
  VECTOR_IRQ:   load = VEC, type = ro, optional = yes, offset = $0004;
}
"#;

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

#[derive(Debug)]
pub struct XemuRun {
    /// Process exit status: the value the program wrote to `$D6CF` before exiting.
    pub exit_code: i32,
    /// Main RAM at exit (384 KiB, flat).
    pub memory: Vec<u8>,
    /// Combined emulator log output, for diagnostics.
    pub log: String,
}

/// Xemu shares preference files between instances, so runs are serialised.
static XEMU_LOCK: Mutex<()> = Mutex::new(());

/// Boots `rom` headless in Xemu and waits for it to exit through `$D6CF`.
pub fn run_rom(xemu: &Path, dir: &Path, rom: &[u8], timeout: Duration) -> Result<XemuRun, String> {
    let _guard = XEMU_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    fs::create_dir_all(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let rom_path = dir.join("kernel.rom");
    let mem_path = dir.join("mem.bin");
    let sd_dir = dir.join("sd");
    fs::create_dir_all(&sd_dir).map_err(|error| format!("create {}: {error}", sd_dir.display()))?;
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
        .stdin(Stdio::null())
        .stdout(log_file.try_clone().map_err(|error| error.to_string())?)
        .stderr(log_file)
        .spawn()
        .map_err(|error| format!("failed to start {}: {error}", xemu.display()))?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
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
    let exit_code = status
        .code()
        .ok_or_else(|| format!("Xemu was terminated by a signal; log: {log}"))?;
    let memory = fs::read(&mem_path)
        .map_err(|error| format!("Xemu wrote no memory dump ({error}); log: {log}"))?;
    Ok(XemuRun {
        exit_code,
        memory,
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
