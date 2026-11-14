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
//! $0002-$00ff   base page (runtime registers, direct-page variables), then
//!               the running program's base-page segment
//! $0200-$1fff   free low RAM (screen, buffers, test output)
//! $2000-...     kernel image: code, data, then BSS
//! ...-$c7ff     program area for loaded o65 programs (see `build_program`)
//!               ($7fff when the kernel has window banks, below)
//! $8000-$bfff   window: the mapped window bank's code (windowed kernels)
//! $c800-$cfff   16-bit stack, growing down from $cfff
//! $d000-$dfff   I/O
//! $e000-$feff   stage-1 loader (ROM window), copied to RAM by stage 2
//! $ff00-$ff1f   trap vector table
//! $fffa-$ffff   NMI / RESET / IRQ vectors
//! ```
//!
//! Stage 1 (in the ROM window) maps BASIC out and copies `$2000-$cfff` from
//! the ROM file. Stage 2 (in RAM) maps the KERNAL ROM out, copies the
//! `$e000` window into the RAM underneath, resets the `MAP` state, clears
//! base page and BSS, and calls `main`.
//!
//! # Window banks
//!
//! Kernel modules can be placed in one of [`WINDOW_BANKS`] window banks (see
//! [`KernelModule`]). Their code runs at `$8000-$bfff` straight out of the
//! ROM image, at linear `$30000 + (bank - 1) * $4000`, while `MAP` points the
//! window there; their data and strings stay in the resident image. Calls
//! into a bank go through resident stubs and [`WINDOW_RUNTIME`], so any
//! module can call any other (see `lower_45gs02::place_in_window`).

pub use crate::lower_45gs02::WINDOW_BANKS;

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

/// Runtime shared by the kernel and loadable programs: the base-page
/// registers generated code uses and the stack-frame routines (see
/// `lower_45gs02`). Each program links its own copy, so a program and the
/// kernel never share scratch state.
pub const RUNTIME: &str = r#".setcpu "45GS02"
.exportzp __m65_acc, __m65_ptr, __m65_farp, __m65_fp, __m65_arg
.export __m65_enter, __m65_leave

.segment "ZEROPAGE"
__m65_acc: .res 4
__m65_ptr: .res 2
__m65_farp: .res 4
__m65_fp: .res 2
__m65_arg: .res 4

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

"#;

/// The runtime's math routines, linked only into images that call them.
pub const MATH_RUNTIME: &str = r#".setcpu "45GS02"
.importzp __m65_acc, __m65_farp, __m65_arg
.export __m65_mul, __m65_divu, __m65_divs

; The math unit (MEGA65 I/O personality).
MULTINA = $d770
MULTINB = $d774
MULTOUT = $d778
DIVOUT_WHOLE = $d76c
MATHBUSY = $d70f                ; bit 7: divider busy, bit 6: multiplier busy

.segment "CODE"
; Math (see lower_45gs02). The operands are ACC and ARG; the results go to
; ACC (product, quotient) and ARG (remainder). The math unit is shared, so
; interrupts are held off while it is in use. A, X, Y and FARP are
; clobbered; Z stays 0.

; ACC = ACC * ARG (low 32 bits). Only the low bytes of the operands that
; the caller set matter for the low bytes of the product.
__m65_mul:
	php
	sei
	jsr __m65_load_operands
@wait:
	bit MATHBUSY
	bvs @wait
	ldx #$03
@out:
	lda MULTOUT,x
	sta __m65_acc,x
	dex
	bpl @out
	plp
	rts

; Writes ACC and ARG to the math unit's inputs.
__m65_load_operands:
	ldx #$03
@in:
	lda __m65_acc,x
	sta MULTINA,x
	lda __m65_arg,x
	sta MULTINB,x
	dex
	bpl @in
	rts

; Unsigned ACC / ARG for X-byte operands (1-4): ACC = quotient, ARG =
; remainder. Dividing by zero gives a quotient of $FFFFFFFF.
__m65_divu:
	php
	sei
@zero:
	cpx #$04
	beq @extended
	lda #$00
	sta __m65_acc,x
	sta __m65_arg,x
	inx
	bra @zero
@extended:
	jsr __m65_divide
	plp
	rts

; Signed ACC / ARG for X-byte operands: the quotient truncates toward zero
; and the remainder has the dividend's sign.
__m65_divs:
	php
	sei
@extend:
	cpx #$04
	beq @extended
	lda __m65_acc-1,x
	asl
	lda #$ff
	adc #$00
	eor #$ff
	sta __m65_acc,x
	lda __m65_arg-1,x
	asl
	lda #$ff
	adc #$00
	eor #$ff
	sta __m65_arg,x
	inx
	bra @extend
@extended:
	lda __m65_acc+3         ; remainder sign
	pha
	eor __m65_arg+3         ; quotient sign
	pha
	ldx #<__m65_acc
	bit __m65_acc+3
	bpl @positive_dividend
	jsr @negate
@positive_dividend:
	ldx #<__m65_arg
	bit __m65_arg+3
	bpl @positive_divisor
	jsr @negate
@positive_divisor:
	jsr __m65_divide
	pla
	bpl @positive_quotient
	ldx #<__m65_acc
	jsr @negate
@positive_quotient:
	pla
	bpl @done
	ldx #<__m65_arg
	jsr @negate
@done:
	plp
	rts
; Negates the 4-byte base-page value at X.
@negate:
	sec
	lda #$00
	sbc $00,x
	sta $00,x
	lda #$00
	sbc $01,x
	sta $01,x
	lda #$00
	sbc $02,x
	sta $02,x
	lda #$00
	sbc $03,x
	sta $03,x
	rts

; 32-bit unsigned ACC / ARG on the math unit: ACC = quotient, ARG =
; remainder (ACC - quotient * ARG, using the multiplier).
__m65_divide:
	jsr __m65_load_operands
@wait:
	bit MATHBUSY
	bmi @wait
	ldx #$03
@quotient:
	lda DIVOUT_WHOLE,x
	sta __m65_farp,x
	dex
	bpl @quotient
	; Only now replace the dividend: the unit recomputes on every write.
	ldx #$03
@multiply:
	lda __m65_farp,x
	sta MULTINA,x           ; quotient * divisor
	dex
	bpl @multiply
@product:
	bit MATHBUSY
	bvs @product
	sec
	lda __m65_acc
	sbc MULTOUT
	sta __m65_arg
	lda __m65_acc+1
	sbc MULTOUT+1
	sta __m65_arg+1
	lda __m65_acc+2
	sbc MULTOUT+2
	sta __m65_arg+2
	lda __m65_acc+3
	sbc MULTOUT+3
	sta __m65_arg+3
	ldx #$03
@copy:
	lda __m65_farp,x
	sta __m65_acc,x
	dex
	bpl @copy
	rts
"#;

/// Window-bank calls (see the module docs and `lower_45gs02::place_in_window`),
/// linked into kernels that have window banks. A stub for routine `foo`
/// in bank `n` is `jsr __m65_farcall / .byte n / .addr __window_foo`.
pub const WINDOW_RUNTIME: &str = r#".setcpu "45GS02"
.importzp __m65_ptr
.export __m65_farcall, __m65_map_window, __m65_window

; Nesting depth of calls into window banks.
WINDOW_DEPTH = 32

.segment "BSS"
__m65_window: .res 1                    ; window bank now mapped (0 = none)
__m65_window_depth: .res 1
; One entry per call in progress: the caller's flags, return address and
; window bank.
__m65_window_p: .res WINDOW_DEPTH
__m65_window_ret_lo: .res WINDOW_DEPTH
__m65_window_ret_hi: .res WINDOW_DEPTH
__m65_window_prev: .res WINDOW_DEPTH
; The callee's results, while the caller's bank is mapped back.
__m65_window_a: .res 1
__m65_window_x: .res 1
__m65_window_y: .res 1

.segment "CODE"
; MAP values for the upper half: bank n runs at $8000-$bfff from linear
; $30000 + (n - 1) * $4000 (offset = linear - $8000; blocks 4 and 5).
__m65_window_map_y: .byte $00, $80, $c0, $00, $40
__m65_window_map_z: .byte $00, $32, $32, $33, $33

; Maps window bank A (0 = none) into $8000-$bfff. Clobbers A/X/Y; Z = 0.
__m65_map_window:
	tax
	ldy __m65_window_map_y,x
	ldz __m65_window_map_z,x
	lda #$00            ; lower half: nothing mapped
	ldx #$00
	map
	eom
	ldz #$00
	rts

; Called by a stub, with the stub's return address (pointing just before
; its bank and target) on top of the caller's, and the caller's arguments
; below. Moves the caller's return address to the window stack, so the
; target sees exactly the stack a direct call would give it, and arranges
; for the target to return to @back, which maps the caller's bank again.
; The window stack is only touched with interrupts off, so handlers can make
; window calls too.
__m65_farcall:
	php
	sei
	ldx __m65_window_depth
	pla
	sta __m65_window_p,x
	pla
	sta __m65_ptr
	pla
	sta __m65_ptr+1
	pla
	sta __m65_window_ret_lo,x
	pla
	sta __m65_window_ret_hi,x
	lda __m65_window
	sta __m65_window_prev,x
	inx
	cpx #WINDOW_DEPTH
	bcs @overflow
	stx __m65_window_depth
	lda #>(@back - 1)   ; the target returns to @back
	pha
	lda #<(@back - 1)
	pha
	ldy #$03            ; RTS into the target: push target - 1
	lda (__m65_ptr),y
	tax
	dey
	lda (__m65_ptr),y
	bne @low
	dex
@low:
	sec
	sbc #$01
	phx
	pha
	dey
	lda (__m65_ptr),y
	sta __m65_window
	jsr __m65_map_window
	ldx __m65_window_depth
	lda __m65_window_p-1,x
	pha
	plp                 ; the caller's interrupt state
	rts
@back:
	sei
	sta __m65_window_a
	stx __m65_window_x
	sty __m65_window_y
	ldx __m65_window_depth
	dex
	stx __m65_window_depth
	lda __m65_window_prev,x
	sta __m65_window
	jsr __m65_map_window
	ldx __m65_window_depth
	lda __m65_window_ret_hi,x
	pha
	lda __m65_window_ret_lo,x
	pha
	lda __m65_window_p,x
	pha
	lda __m65_window_a
	ldx __m65_window_x
	ldy __m65_window_y
	plp
	rts
@overflow:              ; window calls nested too deeply: stop, red border
	lda #$02
	sta $d020
@halt:
	bra @halt
"#;

/// Startup code: the two-stage loader, base-page and BSS clearing,
/// `jsr main`, then exit through Xemu's testing register with `main`'s `A`
/// as the status.
pub const CRT0: &str = r#".setcpu "45GS02"
.importzp __m65_acc, __m65_ptr, __m65_farp, __m65_fp, __m65_arg
.import __m65_enter, __m65_leave
.import main
.import __BSS_RUN__, __BSS_SIZE__, __ZEROPAGE_RUN__, __ZEROPAGE_SIZE__
; Programs load between the end of the kernel's BSS and this address (set by
; the linker configuration: below the window, or below the stack).
.import __m65_program_end
.export machine_program_start, machine_program_end, machine_base_page_start
.export machine_call

__m65_stack_top = $cfff
__m65_image_start = $2000
__m65_image_end = $d000
__m65_rom_linear = $020000

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
	lda #$00            ; MAP: megabyte 0 for both halves, then nothing mapped
	ldx #$0f
	ldy #$00
	ldz #$0f
	map
	ldx #$00
	ldz #$00
	map
	eom
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
; Machine services for the program loader (hellord-exec.mensch).
; First byte of free RAM after the kernel.
machine_program_start:
	lda #<(__BSS_RUN__ + __BSS_SIZE__)
	ldx #>(__BSS_RUN__ + __BSS_SIZE__)
	rts

; End (exclusive) of the program area.
machine_program_end:
	lda #<__m65_program_end
	ldx #>__m65_program_end
	rts

; First base-page byte the kernel does not use.
machine_base_page_start:
	lda #<(__ZEROPAGE_RUN__ + __ZEROPAGE_SIZE__)
	ldx #$00
	rts

; reentrant function machine_call(entry: u16): u16 -- calls `entry` with
; Z = 0 and returns its A/X. The callee keeps its own runtime registers.
machine_call:
	jsr __m65_enter
	ldy #$05            ; the argument, above the saved fp and return address
	lda (__m65_fp),y
	sta __m65_ptr
	iny
	lda (__m65_fp),y
	sta __m65_ptr+1
	jsr @call
	jsr __m65_leave
	rtn #$02
@call:
	jmp (__m65_ptr)

.segment "VECTOR_RESET"
	.addr __m65_reset
"#;

/// ld65 configuration for a 128 KiB MEGA65 ROM image (see the module docs).
/// The kernel's ld65 configuration. A windowed kernel's resident image must
/// end below the window at `$8000`; otherwise it may run up to the I/O area.
/// The four window banks fill the second half of the ROM file.
pub fn linker_config(windowed: bool) -> String {
    let (image_size, gap, program_end) = if windowed {
        (
            "$6000",
            "  GAP:   start = $8000,  size = $5000, file = %O, fill = yes;\n",
            "$8000",
        )
    } else {
        ("$b000", "", "$c800")
    };
    LINKER_CONFIG_TEMPLATE
        .replace("{image_size}", image_size)
        .replace("{gap}", gap)
        .replace("{program_end}", program_end)
}

const LINKER_CONFIG_TEMPLATE: &str = r#"MEMORY {
  ZP:    start = $0002,  size = $00fe, type = rw;
  LOW:   start = $0000,  size = $2000, file = %O, fill = yes;
  IMAGE: start = $2000,  size = {image_size}, file = %O, fill = yes;
{gap}  IO:    start = $d000,  size = $1000, file = %O, fill = yes;
  ROM:   start = $e000,  size = $1f00, file = %O, fill = yes;
  TRAP:  start = $ff00,  size = $0020, file = %O, fill = yes;
  TAIL:  start = $ff20,  size = $00da, file = %O, fill = yes;
  VEC:   start = $fffa,  size = $0006, file = %O, fill = yes;
  WIN1:  start = $8000,  size = $4000, file = %O, fill = yes;
  WIN2:  start = $8000,  size = $4000, file = %O, fill = yes;
  WIN3:  start = $8000,  size = $4000, file = %O, fill = yes;
  WIN4:  start = $8000,  size = $4000, file = %O, fill = yes;
}
SEGMENTS {
  ZEROPAGE:     load = ZP,    type = zp, define = yes;
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
  WINDOW1:      load = WIN1, type = ro, optional = yes;
  WINDOW2:      load = WIN2, type = ro, optional = yes;
  WINDOW3:      load = WIN3, type = ro, optional = yes;
  WINDOW4:      load = WIN4, type = ro, optional = yes;
}
SYMBOLS {
  __m65_program_end: type = export, value = {program_end};
}
"#;

/// Base-page bytes the runtime registers in [`RUNTIME`] take.
pub const RUNTIME_BASE_PAGE_BYTES: u16 = 16;

/// Base-page bytes available to kernel Mensch code: `$02-$FF` minus the
/// runtime registers.
pub const BASE_PAGE_BUDGET: u16 = 0xfe - RUNTIME_BASE_PAGE_BYTES;

/// Base-page bytes available to a loadable program's Mensch code. A program
/// gets the base page the kernel leaves free, so it is kept small.
pub const PROGRAM_BASE_PAGE_BUDGET: u16 = 0x30;

/// Compiles Mensch modules for the MEGA65 kernel with one shared base-page
/// budget. Returns `(module file stem, assembly)` pairs ready for [`build_rom`].
pub fn compile_modules(sources: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    compile_modules_with_budget(sources, BASE_PAGE_BUDGET)
}

/// A kernel module to compile, and the window bank its code goes in
/// (`None` keeps it in the resident image).
#[derive(Debug, Clone)]
pub struct KernelModule {
    pub name: String,
    pub source: String,
    pub window: Option<u8>,
}

/// Compiles kernel modules, placing each in its window bank. Returns
/// `(module file stem, assembly)` pairs ready for [`build_rom`].
pub fn compile_kernel_modules(modules: &[KernelModule]) -> Result<Vec<(String, String)>, String> {
    compile_with_budget(modules, BASE_PAGE_BUDGET)
}

/// Compiles the Mensch modules of a loadable program (see [`build_program`]).
pub fn compile_program_modules(
    sources: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    compile_modules_with_budget(sources, PROGRAM_BASE_PAGE_BUDGET)
}

fn compile_modules_with_budget(
    sources: &[(String, String)],
    budget: u16,
) -> Result<Vec<(String, String)>, String> {
    let modules: Vec<KernelModule> = sources
        .iter()
        .map(|(name, source)| KernelModule {
            name: name.clone(),
            source: source.clone(),
            window: None,
        })
        .collect();
    compile_with_budget(&modules, budget)
}

fn compile_with_budget(
    modules: &[KernelModule],
    budget: u16,
) -> Result<Vec<(String, String)>, String> {
    let mut assignments = Vec::new();
    let mut out = Vec::new();
    for module in modules {
        let name = &module.name;
        let used: u16 = assignments
            .iter()
            .map(|assignment: &crate::storage::StorageAssignment| assignment.direct_page_bytes)
            .sum();
        let ir = crate::frontend::compile_source(&module.source)
            .map_err(|error| format!("{name}: {error}"))?;
        let (mut lowered, assignment) =
            crate::lower_45gs02::lower_module_with_storage_budget(&ir, budget.saturating_sub(used))
                .map_err(|error| format!("{name}: {error}"))?;
        if let Some(bank) = module.window {
            crate::lower_45gs02::place_in_window(&mut lowered, bank)?;
        }
        assignments.push(assignment);
        out.push((name.clone(), crate::ca65::emit(&lowered)));
    }
    crate::storage::validate_direct_page_budget(&assignments, budget)
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// Assembles and links generated modules with the runtime into a ROM image.
///
/// `modules` are `(name, ca65 source)` pairs. Intermediate files go in `dir`.
pub fn build_rom(dir: &Path, modules: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    let windowed = modules
        .iter()
        .any(|(_, asm)| asm.contains(crate::lower_45gs02::FARCALL_ROUTINE));
    let mut runtime = vec![("runtime", RUNTIME), ("crt0", CRT0)];
    if windowed {
        runtime.push(("window", WINDOW_RUNTIME));
    }
    let image = link(dir, "kernel", &linker_config(windowed), &runtime, modules)?;
    if image.len() != ROM_SIZE {
        return Err(format!(
            "linked ROM is {} bytes, expected {ROM_SIZE}",
            image.len()
        ));
    }
    Ok(image)
}

/// ld65 configuration for a loadable program: a relocatable o65 file whose
/// text, data, BSS and base-page segments the kernel's loader places.
pub const PROGRAM_LINKER_CONFIG: &str = r#"MEMORY {
  ZP:   start = $0000, size = $0100, type = rw;
  TEXT: start = $0000, size = $c000, type = ro;
  DATA: start = $0000, size = $c000, type = rw;
  BSS:  start = $0000, size = $c000, type = rw;
}
SEGMENTS {
  ZEROPAGE:    load = ZP,   type = zp;
  CODE:        load = TEXT, type = ro;
  LOWCODE:     load = TEXT, type = ro, optional = yes;
  RODATA:      load = TEXT, type = ro, optional = yes;
  MENSCH_META: load = TEXT, type = ro, optional = yes;
  DATA:        load = DATA, type = rw, optional = yes;
  BSS:         load = BSS,  type = bss, optional = yes;
}
FILES { %O: format = o65; }
FORMATS { o65: type = small, os = lunix, export = main; }
"#;

/// Links compiled program modules into an o65 executable for Hellord's
/// `RUN`. One module must export `main`; the loader calls it with `JSR` and
/// takes `A` as the exit status. Programs reach the kernel only through the
/// trap table, so the file has no undefined references.
pub fn build_program(dir: &Path, modules: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    link(
        dir,
        "program",
        PROGRAM_LINKER_CONFIG,
        &[("runtime", RUNTIME)],
        modules,
    )
}

fn link(
    dir: &Path,
    output: &str,
    config: &str,
    runtime: &[(&str, &str)],
    modules: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    fs::create_dir_all(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let cfg = dir.join(format!("{output}.cfg"));
    write(&cfg, config)?;

    // The math routines are only linked when some module calls them.
    let math = modules.iter().any(|(_, source)| {
        crate::lower_45gs02::MATH_ROUTINES
            .iter()
            .any(|routine| source.contains(routine))
    });
    let math = math.then_some(("math", MATH_RUNTIME));
    let mut objects = Vec::new();
    for (name, source) in runtime.iter().chain(math.iter()).chain(modules) {
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

    let file = dir.join(format!("{output}.bin"));
    run(Command::new("ld65")
        .arg("-C")
        .arg(&cfg)
        .arg("-m")
        .arg(dir.join(format!("{output}.map")))
        .arg("-o")
        .arg(&file)
        .args(&objects))?;
    fs::read(&file).map_err(|error| format!("read {}: {error}", file.display()))
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
