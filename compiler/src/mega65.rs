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
//!
//! A kernel with window banks is a *split* kernel: its resident image ends
//! below the window at `$8000`, a small resident core in `$C000-$C7FF` and
//! `$E000-$FEFF` handles every mapping change, and programs run in a bank
//! of their own (linear `$40000`, seen at `$0000-$BFFF`), entering the
//! kernel through the core's trap and IRQ entries. See `ABI.md`.

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
/// Program services for a kernel without window banks: programs load into
/// bank-0 RAM between the end of the kernel's BSS and the stack, and share
/// the base page with the kernel.
pub const BANK0_SERVICES: &str = r#".setcpu "45GS02"
.importzp __m65_ptr, __m65_fp
.import __m65_enter, __m65_leave
.import __BSS_RUN__, __BSS_SIZE__, __ZEROPAGE_RUN__, __ZEROPAGE_SIZE__
.export machine_program_start, machine_program_end, machine_base_page_start
.export machine_call, machine_caller_bank, machine_program_bank, __m65_mode_init

__m65_program_end = $c800

.segment "CODE"
__m65_mode_init:
	rts

; Everything runs in bank 0.
machine_caller_bank:
machine_program_bank:
	lda #$00
	tax
	rts

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

"#;

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

/// The resident core of a split kernel (one with window banks): memory
/// mapping, calls into window banks, trap and interrupt entry, and running
/// programs in their own bank. It lives in the always-visible `$C000-$C7FF`
/// (state) and `$E000-$FEFF` (code), linked into kernels with window banks.
///
/// Two mapping registers describe the CPU's view: the *lower* bank shown at
/// `$0000-$7FFF` (0 = the kernel's bank 0, 1 = the program bank at linear
/// `$40000`) and the *window* shown at `$8000-$BFFF` (0 = none, 1-4 = kernel
/// window banks, 5 = the program bank's `$8000-$BFFF`). Every switch goes
/// through one routine, `__m65_switch`, which keeps the caller's flags,
/// return address and mapping on a resident stack, so the target sees the
/// stack a direct call would give it, and restores them when it returns.
///
/// - Window stubs (`jsr __m65_farcall / .byte n / .addr body`) select window
///   `n` and keep the lower bank.
/// - The trap table at `$FF00` points at `__m65_trap_N`, which select the
///   kernel's lower bank and call the kernel's dispatcher for family N
///   (the modules' trap vectors, linked at `$FEC0`).
/// - The IRQ vector points at `__m65_irq`, which maps the kernel in, runs
///   the kernel's handler (the modules' IRQ vector, linked at `$FEE4`) and
///   maps the interrupted code back.
/// - `machine_call(entry)` maps the program bank in (lower bank 1, window 5)
///   and calls `entry`.
pub const WINDOW_RUNTIME: &str = r#".setcpu "45GS02"
.importzp __m65_ptr, __m65_fp
.import __m65_enter, __m65_leave
.export __m65_farcall, __m65_mode_init, __m65_lower, __m65_window
.export machine_program_start, machine_program_end, machine_base_page_start
.export machine_call, machine_caller_bank, machine_program_bank

; Nesting depth of mapping switches (window calls, traps, program calls).
SWITCH_DEPTH = 32
KERNEL_TRAPS = $fec0
KERNEL_IRQ = $fee4
PROGRAM_LOWER = 1
PROGRAM_WINDOW = 5

.segment "HIBSS"
__m65_lower: .res 1                     ; lower bank now mapped
__m65_window: .res 1                    ; window bank now mapped
__m65_depth: .res 1
__m65_next_lower: .res 1                ; the switch being made
__m65_next_window: .res 1
__m65_next_lo: .res 1
__m65_next_hi: .res 1
; One entry per switch in progress: the caller's flags, return address and
; mapping.
__m65_switch_p: .res SWITCH_DEPTH
__m65_switch_lo: .res SWITCH_DEPTH
__m65_switch_hi: .res SWITCH_DEPTH
__m65_switch_window: .res SWITCH_DEPTH
__m65_switch_lower: .res SWITCH_DEPTH
; The callee's results, while the caller's mapping comes back.
__m65_switch_a: .res 1
__m65_switch_x: .res 1
__m65_switch_y: .res 1

.segment "HICODE"
; MAP values. Lower bank n: A, X. Window bank n: Y, Z (bank n of 1-4 runs
; from linear $30000 + (n - 1) * $4000; offset = linear - $8000).
__m65_lower_map_a: .byte $00, $00
__m65_lower_map_x: .byte $00, $f4       ; blocks 0-3 at linear $40000
__m65_window_map_y: .byte $00, $80, $c0, $00, $40, $00
__m65_window_map_z: .byte $00, $32, $32, $33, $33, $34

; Applies __m65_lower and __m65_window. Clobbers A/X/Y; Z = 0.
__m65_apply_map:
	ldx __m65_window
	ldy __m65_window_map_y,x
	ldz __m65_window_map_z,x
	ldx __m65_lower
	lda __m65_lower_map_x,x
	pha
	lda __m65_lower_map_a,x
	plx
	map
	eom
	ldz #$00
	rts

__m65_mode_init:
	lda #$00
	sta __m65_lower
	sta __m65_window
	sta __m65_depth
	jmp __m65_apply_map

; Window stub entry: the stub's return address points just before its bank
; and target.
__m65_farcall:
	php
	sei
	pla
	sta __m65_switch_a      ; (scratch: interrupts are off)
	pla
	sta __m65_ptr           ; kernel code only: this is the kernel's base page
	pla
	sta __m65_ptr+1
	lda __m65_switch_a
	pha
	ldy #$01
	lda (__m65_ptr),y
	sta __m65_next_window
	iny
	lda (__m65_ptr),y
	sta __m65_next_lo
	iny
	lda (__m65_ptr),y
	sta __m65_next_hi
	lda __m65_lower
	sta __m65_next_lower
	jmp __m65_switch

; Trap entry for family X/2: the kernel's lower bank, the window kept.
__m65_trapcall:
	php
	sei
	lda KERNEL_TRAPS,x
	sta __m65_next_lo
	lda KERNEL_TRAPS+1,x
	sta __m65_next_hi
	lda #$00
	sta __m65_next_lower
	lda __m65_window
	sta __m65_next_window
	jmp __m65_switch

; Calls the program entry point in __m65_ptr with the program bank mapped.
__m65_usercall:
	php
	sei
	lda __m65_ptr
	sta __m65_next_lo
	lda __m65_ptr+1
	sta __m65_next_hi
	lda #PROGRAM_LOWER
	sta __m65_next_lower
	lda #PROGRAM_WINDOW
	sta __m65_next_window
	jmp __m65_switch

; With interrupts off and the caller's flags on top of its return address
; (and its arguments below), switches to __m65_next_* and enters
; __m65_next_lo/hi, which returns to @back.
__m65_switch:
	ldx __m65_depth
	pla
	sta __m65_switch_p,x
	pla
	sta __m65_switch_lo,x
	pla
	sta __m65_switch_hi,x
	lda __m65_window
	sta __m65_switch_window,x
	lda __m65_lower
	sta __m65_switch_lower,x
	inx
	cpx #SWITCH_DEPTH
	bcs @overflow
	stx __m65_depth
	lda #>(@back - 1)   ; the target returns to @back
	pha
	lda #<(@back - 1)
	pha
	ldx __m65_next_hi   ; RTS into the target: push target - 1
	lda __m65_next_lo
	bne @low
	dex
@low:
	sec
	sbc #$01
	phx
	pha
	lda __m65_next_lower
	sta __m65_lower
	lda __m65_next_window
	sta __m65_window
	jsr __m65_apply_map
	ldx __m65_depth
	lda __m65_switch_p-1,x
	pha
	plp                 ; the caller's interrupt state
	rts
@back:
	sei
	sta __m65_switch_a
	stx __m65_switch_x
	sty __m65_switch_y
	ldx __m65_depth
	dex
	stx __m65_depth
	lda __m65_switch_window,x
	sta __m65_window
	lda __m65_switch_lower,x
	sta __m65_lower
	jsr __m65_apply_map
	ldx __m65_depth
	lda __m65_switch_hi,x
	pha
	lda __m65_switch_lo,x
	pha
	lda __m65_switch_p,x
	pha
	lda __m65_switch_a
	ldx __m65_switch_x
	ldy __m65_switch_y
	plp
	rts
@overflow:              ; switches nested too deeply: stop, red border
	lda #$02
	sta $d020
@halt:
	bra @halt

; IRQ entry: maps the kernel in around its handler. The handler returns
; with RTI into @restore through a frame pushed here.
__m65_irq:
	pha
	phx
	phy
	phz
	lda __m65_lower
	pha
	lda #$00
	sta __m65_lower
	jsr __m65_apply_map
	lda #>@restore
	pha
	lda #<@restore
	pha
	php
	jmp (KERNEL_IRQ)
@restore:
	pla
	sta __m65_lower
	jsr __m65_apply_map
	plz
	ply
	plx
	pla
	rti

.macro trap_entry family
	.ident(.sprintf("__m65_trap_%d", family)):
	ldx #family * 2
	jmp __m65_trapcall
.endmacro
.repeat 16, family
	trap_entry family
.endrepeat

.segment "M65_TRAPS"
.repeat 16, family
	.addr .ident(.sprintf("__m65_trap_%d", family))
.endrepeat

.segment "M65_IRQ"
	.addr __m65_irq

.segment "CODE"
; Machine services for the program loader (hellord-exec.mensch). Programs
; run in their own bank: $0200-$bfff of linear $40000, with their own base
; page (from $02).
machine_program_start:
	lda #$00
	ldx #$02
	rts

machine_program_end:
	lda #$00
	ldx #$c0
	rts

machine_base_page_start:
	lda #$02
	ldx #$00
	rts

machine_program_bank:
	lda #PROGRAM_LOWER
	ldx #$00
	rts

; The lower bank of whoever made the innermost switch: the program (1) when
; called from a trap dispatcher serving a program, else the kernel (0).
machine_caller_bank:
	ldx __m65_depth
	beq @kernel
	lda __m65_switch_lower-1,x
	ldx #$00
	rts
@kernel:
	lda #$00
	rts

; reentrant function machine_call(entry: u16): u16 -- runs the program's
; entry point in the program bank and returns its A/X.
machine_call:
	jsr __m65_enter
	ldy #$05            ; the argument, above the saved fp and return address
	lda (__m65_fp),y
	sta __m65_ptr
	iny
	lda (__m65_fp),y
	sta __m65_ptr+1
	jsr __m65_usercall
	jsr __m65_leave
	rtn #$02
"#;

/// Startup code: the two-stage loader, base-page and BSS clearing,
/// `jsr main`, then exit through Xemu's testing register with `main`'s `A`
/// as the status.
pub const CRT0: &str = r#".setcpu "45GS02"
.importzp __m65_acc, __m65_ptr, __m65_farp, __m65_fp, __m65_arg
.import __m65_enter, __m65_leave
.import main
.import __BSS_RUN__, __BSS_SIZE__
.import __m65_mode_init

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
	jsr __m65_mode_init
	jsr main
	sta $d6cf           ; exit status (Xemu -testing)
	lda #$42
	sta $d6cf           ; request exit
@hang:
	bra @hang

.segment "VECTOR_RESET"
	.addr __m65_reset
"#;

/// ld65 configuration for a 128 KiB MEGA65 ROM image (see the module docs).
/// The kernel's ld65 configuration. A windowed kernel's resident image must
/// end below the window at `$8000`; otherwise it may run up to the I/O area.
/// The four window banks fill the second half of the ROM file.
pub fn linker_config(windowed: bool) -> String {
    if windowed {
        return windowed_linker_config();
    }
    LINKER_CONFIG_TEMPLATE
        .replace("{image_size}", "$b000")
        .replace("{gap}", "")
}

/// The split kernel's configuration: the resident image below the window,
/// the resident core's state at `$C000` and code in the `$E000` area, the
/// modules' trap and IRQ vectors in a kernel table at `$FEC0` (the CPU's
/// point at the core's entry routines), and the four window banks in the
/// second half of the ROM file.
fn windowed_linker_config() -> String {
    let mut traps = String::new();
    for family in 0..16 {
        traps.push_str(&format!(
            "  VECTOR_TRAP_{family:02X}: load = KTRAP, type = ro, optional = yes, offset = ${:04x};\n",
            family * 2
        ));
    }
    format!(
        r#"MEMORY {{
  ZP:    start = $0002,  size = $00fe, type = rw;
  LOW:   start = $0000,  size = $2000, file = %O, fill = yes;
  IMAGE: start = $2000,  size = $6000, file = %O, fill = yes;
  GAP:   start = $8000,  size = $5000, file = %O, fill = yes;
  HIRAM: start = $c000,  size = $0800, type = rw;
  IO:    start = $d000,  size = $1000, file = %O, fill = yes;
  ROM:   start = $e000,  size = $1ec0, file = %O, fill = yes;
  KTRAP: start = $fec0,  size = $0020, file = %O, fill = yes;
  KVEC:  start = $fee0,  size = $0006, file = %O, fill = yes;
  KTAIL: start = $fee6,  size = $001a, file = %O, fill = yes;
  TRAP:  start = $ff00,  size = $0020, file = %O, fill = yes;
  TAIL:  start = $ff20,  size = $00da, file = %O, fill = yes;
  VEC:   start = $fffa,  size = $0006, file = %O, fill = yes;
  WIN1:  start = $8000,  size = $4000, file = %O, fill = yes;
  WIN2:  start = $8000,  size = $4000, file = %O, fill = yes;
  WIN3:  start = $8000,  size = $4000, file = %O, fill = yes;
  WIN4:  start = $8000,  size = $4000, file = %O, fill = yes;
}}
SEGMENTS {{
  ZEROPAGE:     load = ZP,    type = zp, define = yes;
  KERNEL_START: load = IMAGE, type = ro;
  LOWCODE:      load = IMAGE, type = ro, optional = yes;
  CODE:         load = IMAGE, type = ro;
  RODATA:       load = IMAGE, type = ro, optional = yes;
  MENSCH_META:  load = IMAGE, type = ro, optional = yes;
  BANK1:        load = IMAGE, type = ro, optional = yes;
  BSS:          load = IMAGE, type = bss, define = yes;
  HIBSS:        load = HIRAM, type = bss;
  STARTUP:      load = ROM,   type = ro;
  HICODE:       load = ROM,   type = ro;
{traps}  VECTOR_NMI:   load = KVEC,  type = ro, optional = yes, offset = $0000;
  VECTOR_IRQ:   load = KVEC,  type = ro, optional = yes, offset = $0004;
  M65_TRAPS:    load = TRAP,  type = ro;
  VECTOR_RESET: load = VEC,   type = ro, offset = $0002;
  M65_IRQ:      load = VEC,   type = ro, offset = $0004;
  WINDOW1:      load = WIN1,  type = ro, optional = yes;
  WINDOW2:      load = WIN2,  type = ro, optional = yes;
  WINDOW3:      load = WIN3,  type = ro, optional = yes;
  WINDOW4:      load = WIN4,  type = ro, optional = yes;
}}
"#
    )
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
  HIGH:  start = $10000, size = $10000, file = %O, fill = yes;
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
    runtime.push(if windowed {
        ("window", WINDOW_RUNTIME)
    } else {
        ("bank0", BANK0_SERVICES)
    });
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
