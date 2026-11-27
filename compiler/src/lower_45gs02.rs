//! Lowering from Mensch IR to 45GS02 (MEGA65) assembly.
//!
//! The 45GS02 accumulator is always 8 bits wide, so every IR operation is
//! lowered byte by byte: a value of width N is N bytes at some location, and
//! add/sub/compare/move walk those bytes. The IR's virtual accumulator
//! (`Value::Accumulator`) lives in the base-page runtime register
//! [`ACC_SYMBOL`]; function results come back in `A` (byte 0), `X` (byte 1),
//! `Y` (byte 2) and `ACC+3` (byte 3).
//!
//! Multiplication and division call the runtime's math routines
//! ([`MATH_ROUTINES`]) with the operands in `ACC` and [`ARG_SYMBOL`]; they
//! use the MEGA65's hardware multiplier and divider.
//!
//! # Stack frames
//!
//! The runtime runs the CPU with a 16-bit stack pointer. A routine with stack
//! storage (every `reentrant` routine, and any routine whose scratch values
//! spilled out of base page) addresses its frame through the base-page frame
//! pointer [`FP_SYMBOL`] with `(fp),y`, so argument pushes never move local
//! offsets. Relative to `fp`, a frame of `F` local bytes looks like:
//!
//! ```text
//! fp+1 .. fp+F        locals and spilled scratch (zeroed on entry)
//! fp+F+1, fp+F+2      caller's fp (lo, hi)
//! fp+F+3, fp+F+4      return address
//! fp+F+5 ..           stack arguments, first parameter lowest
//! ```
//!
//! The frame is built and released by the runtime routines [`ENTER_ROUTINE`]
//! and [`LEAVE_ROUTINE`] (Z = `F`). After lowering, each routine is cleaned up
//! by [`crate::optimize_45gs02`].
//!
//! The storage pass numbers stack slots for the 65816's `offset,s` view,
//! where parameters start at `F+3`; offsets above `F` are shifted by the two
//! saved-fp bytes. Reentrant callees pop their own arguments with the
//! 65CE02's `RTN #n`, so the caller's `CleanupStack` emits nothing and the
//! A/X/Y result registers survive until the caller stores them.
//!
//! Not yet supported (lowering returns an error naming the construct):
//! COP trap frames and absolute addresses above `$FFFF`.

use std::collections::HashSet;
use std::fmt;

use crate::ir::{
    Abi, Address, ArgRegister, BinaryOp, Const, GlobalPlacement, IrInst, Label, Module, Routine,
    RoutineKind, Type, Value, Width,
};
use crate::optimize_45gs02::optimize_routine_with;
use crate::storage::{
    apply_storage, assign_storage_with_options, StorageAssignment, StorageOptions,
};
use crate::target_45gs02::{AsmModule, AsmRoutine, Inst, Op, Operand};

/// Base-page virtual accumulator (4 bytes), defined by the runtime.
pub const ACC_SYMBOL: &str = "__m65_acc";
/// Base-page scratch near pointer (2 bytes), defined by the runtime.
pub const PTR_SYMBOL: &str = "__m65_ptr";
/// Base-page scratch flat pointer (4 bytes, top byte kept zero), defined by the runtime.
pub const FARP_SYMBOL: &str = "__m65_farp";
/// Base-page frame pointer (2 bytes), defined by the runtime.
pub const FP_SYMBOL: &str = "__m65_fp";
/// Base-page second operand of the math routines (4 bytes).
pub const ARG_SYMBOL: &str = "__m65_arg";
/// Base-page temporaries of reentrant routines (see `StorageOptions`).
pub const TMP_SYMBOL: &str = "__m65_tmp";
pub const TMP_BYTES: u16 = 8;

pub const RUNTIME_DIRECT_SYMBOLS: [&str; 6] = [
    ACC_SYMBOL,
    PTR_SYMBOL,
    FARP_SYMBOL,
    FP_SYMBOL,
    ARG_SYMBOL,
    TMP_SYMBOL,
];

/// Runtime routine that builds a stack frame (Z = local bytes).
pub const ENTER_ROUTINE: &str = "__m65_enter";
/// Runtime routine that releases a stack frame (Z = local bytes).
pub const LEAVE_ROUTINE: &str = "__m65_leave";
/// `ACC = ACC * ARG` (the low 32 bits).
pub const MUL_ROUTINE: &str = "__m65_mul";
/// Unsigned `ACC / ARG` for X-byte operands: quotient in ACC, remainder in
/// ARG (both 32-bit).
pub const DIVU_ROUTINE: &str = "__m65_divu";
/// Signed `ACC / ARG`, like [`DIVU_ROUTINE`]: the quotient truncates toward
/// zero and the remainder has the dividend's sign.
pub const DIVS_ROUTINE: &str = "__m65_divs";
/// Runtime routines that take their operands in ACC and ARG.
pub const MATH_ROUTINES: [&str; 3] = [MUL_ROUTINE, DIVU_ROUTINE, DIVS_ROUTINE];
/// Runtime routines generated code may call.
pub const RUNTIME_ROUTINES: [&str; 5] = [
    ENTER_ROUTINE,
    LEAVE_ROUTINE,
    MUL_ROUTINE,
    DIVU_ROUTINE,
    DIVS_ROUTINE,
];

/// Fixed address of the trap vector table: family `n`'s dispatcher address
/// is the word at `TRAP_TABLE + 2n`. Part of the MEGA65 kernel ABI.
pub const TRAP_TABLE: u16 = 0xff00;

/// Bytes between the locals and the return address (the saved fp).
const SAVED_FP_BYTES: u16 = 2;

/// Wide values never need base page here: far pointers are copied into
/// the runtime's flat pointer before use.
/// Reentrant routines keep short-lived temporaries in `__m65_tmp`, which
/// interrupt handlers preserve.
const STORAGE_OPTIONS: StorageOptions = StorageOptions {
    wide_scratch_direct: false,
    temp_registers: Some((TMP_SYMBOL, TMP_BYTES)),
    cheap_frames: false,
};

pub fn lower_module_with_storage(
    module: &Module,
) -> Result<(AsmModule, StorageAssignment), LowerError> {
    lower_with_assignment(
        module,
        assign_storage_with_options(module, None, STORAGE_OPTIONS),
    )
}

pub fn lower_module_with_storage_budget(
    module: &Module,
    direct_page_budget: u16,
) -> Result<(AsmModule, StorageAssignment), LowerError> {
    let mut module = module.clone();
    crate::inline::inline_small_routines(&mut module);
    crate::narrow::narrow_module(&mut module);
    lower_with_assignment(
        &module,
        assign_storage_with_options(&module, Some(direct_page_budget), STORAGE_OPTIONS),
    )
}

fn lower_with_assignment(
    module: &Module,
    assignment: StorageAssignment,
) -> Result<(AsmModule, StorageAssignment), LowerError> {
    let assigned = apply_storage(module, &assignment);
    let mut out = AsmModule::declarations_from(&assigned);
    for symbol in RUNTIME_DIRECT_SYMBOLS {
        if !out
            .external_direct_symbols
            .iter()
            .any(|name| name == symbol)
        {
            out.external_direct_symbols.push(symbol.to_owned());
        }
    }
    // Functions with 1- or 2-byte results return with N/Z describing A
    // (`load_result` loads A last; so does `__m65_leave`).
    let flag_routines: HashSet<String> = assigned
        .routines
        .iter()
        .filter(|routine| {
            matches!(&routine.kind, RoutineKind::Function { result } if result.size_bytes() <= 2)
                && !matches!(routine.abi, Abi::Interrupt)
                && !routine.result_in_carry
        })
        .map(|routine| routine.name.clone())
        .collect();
    // Memory the optimiser may access 32 bits at a time: this module's
    // ordinary variables (not at a fixed address, not volatile).
    let plain_memory: HashSet<String> = assigned
        .globals
        .iter()
        .filter(|global| matches!(global.placement, GlobalPlacement::Static) && !global.volatile)
        .map(|global| global.name.clone())
        .collect();
    for routine in &assigned.routines {
        out.routines.push(lower_routine(routine, &flag_routines, &plain_memory)?);
    }
    for helper in RUNTIME_ROUTINES {
        let called = out.routines.iter().any(|routine| {
            routine.insts.iter().any(
                |inst| matches!(inst, Inst::Op(Op::Jsr, Operand::Label(name)) if name == helper),
            )
        });
        if called && !out.external_routines.iter().any(|name| name == helper) {
            out.external_routines.push(helper.to_owned());
        }
    }
    out.add_storage_symbols(&assigned, &assignment);
    out.rename_clashing_symbols();
    Ok((out, assignment))
}

/// Window banks a kernel image can hold (see `mega65::WINDOW_RUNTIME`).
pub const WINDOW_BANKS: u8 = 4;
/// Runtime routine behind every window stub.
pub const FARCALL_ROUTINE: &str = "__m65_farcall";

/// The window-bank body of a routine whose resident stub keeps its name.
pub fn window_body_name(routine: &str) -> String {
    format!("__window_{routine}")
}

/// Moves a lowered module's code into window bank `bank`: the code runs at
/// `$8000-$BFFF` while that bank is mapped there. Everything else stays
/// resident: its data, its strings, and a 6-byte stub for each routine that
/// can be reached from outside the module (exported, address taken, or a
/// trap vector), which maps the bank for the call. Calls inside the module
/// go straight to the bodies, and pointers to its routines point at stubs.
pub fn place_in_window(module: &mut AsmModule, bank: u8) -> Result<(), String> {
    if !(1..=WINDOW_BANKS).contains(&bank) {
        return Err(format!(
            "module {}: window bank {bank} is not in 1-{WINDOW_BANKS}",
            module.name
        ));
    }
    let names: Vec<String> = module
        .routines
        .iter()
        .map(|routine| routine.name.clone())
        .collect();
    let mut reached: Vec<String> = module
        .routines
        .iter()
        .filter(|routine| routine.export)
        .map(|routine| routine.name.clone())
        .collect();
    let mut reach = |name: &str| {
        if names.iter().any(|known| known == name) && !reached.iter().any(|seen| seen == name) {
            reached.push(name.to_owned());
        }
    };
    for routine in &module.routines {
        for inst in &routine.insts {
            if let Inst::Op(_, Operand::Immediate(text)) = inst {
                if let Some(expression) = text.strip_prefix(['<', '>', '^']) {
                    let expression = expression.trim_start_matches('(').trim_end_matches(')');
                    reach(expression.split('+').next().unwrap_or(expression));
                }
            }
        }
    }
    for vector in &module.vectors {
        if !names.contains(&vector.routine) {
            continue;
        }
        if !vector.name.starts_with("trap_") {
            return Err(format!(
                "module {}: interrupt vector {} needs resident code; {} cannot live in a window bank",
                module.name, vector.name, vector.routine
            ));
        }
        reach(&vector.routine);
    }

    let mut stubs = Vec::new();
    for routine in &mut module.routines {
        for inst in &mut routine.insts {
            if let Inst::Op(Op::Jsr, Operand::Label(target)) = inst {
                if reached.contains(target) {
                    *target = window_body_name(target);
                }
            }
        }
        if reached.contains(&routine.name) {
            let body = window_body_name(&routine.name);
            let mut stub = AsmRoutine::new(routine.name.clone());
            stub.export = routine.export;
            stub.segment = Some("CODE".to_owned());
            stub.insts = vec![Inst::Raw(format!(
                "jsr {FARCALL_ROUTINE}\n\t.byte {bank}\n\t.addr {body}"
            ))];
            stubs.push(stub);
            routine.name = body;
            routine.export = false;
        }
    }
    if !stubs.is_empty() {
        module.external_routines.push(FARCALL_ROUTINE.to_owned());
    }
    module.routines.extend(stubs);
    module
        .metadata
        .retain(|entry| entry.key != "code_segment" && entry.key != "data_segment");
    module.metadata.push(crate::asm::ModuleMetadata {
        key: "code_segment".to_owned(),
        value: format!("WINDOW{bank}"),
    });
    module.metadata.push(crate::asm::ModuleMetadata {
        key: "data_segment".to_owned(),
        value: "RODATA".to_owned(),
    });
    Ok(())
}

fn lower_routine(
    routine: &Routine,
    flag_routines: &HashSet<String>,
    plain_memory: &HashSet<String>,
) -> Result<AsmRoutine, LowerError> {
    let mut lowerer = RoutineLowerer::new(routine);
    if lowerer.frame.is_some_and(|frame| frame > 0xff) {
        return Err(lowerer.unsupported(format!(
            "a {}-byte stack frame (at most 255)",
            routine.stack_frame_bytes
        )));
    }
    // Register arguments are stored before the prologue, whose frame setup
    // uses A, X and Y.
    let arrivals = routine
        .body
        .iter()
        .take_while(|inst| {
            matches!(
                inst,
                IrInst::Move {
                    src: Value::Register(_),
                    ..
                }
            )
        })
        .count();
    for inst in &routine.body[..arrivals] {
        lowerer.lower_inst(inst)?;
    }
    lowerer.prologue();
    for inst in &routine.body[arrivals..] {
        lowerer.lower_inst(inst)?;
    }
    if lowerer.pending_result.is_some() {
        return Err(lowerer.unsupported("a call result that is never stored"));
    }
    if lowerer.is_interrupt() {
        lowerer.save_runtime_registers();
    }
    if lowerer.shared_epilogue_used {
        let label = lowerer.shared_epilogue_label();
        lowerer.out.push(Inst::Label(label));
        lowerer.epilogue();
    }
    let mut out = AsmRoutine::new(routine.name.clone());
    out.export = routine.export;
    // Only interrupt handlers can start with Z non-zero.
    let z_zero_on_entry = !matches!(routine.abi, Abi::Interrupt);
    out.insts = optimize_routine_with(
        lowerer.out,
        z_zero_on_entry,
        flag_routines,
        routine.result_in_carry,
        Some(match &routine.kind {
            RoutineKind::Function { result } if !routine.result_in_carry => result.size_bytes(),
            _ => 0,
        }),
        plain_memory,
    );
    Ok(out)
}

struct RoutineLowerer<'a> {
    routine: &'a Routine,
    out: Vec<Inst>,
    next_label: usize,
    /// Local frame bytes (`F`), or `None` when the routine has no frame.
    frame: Option<u16>,
    /// Argument bytes the routine pops on return (`RTN #n`).
    param_bytes: u16,
    /// Width of a call result still held in A/X/Y (see `IrInst::CallFunction`).
    pending_result: Option<Width>,
    /// Whether some return jumped to the routine's single shared epilogue.
    shared_epilogue_used: bool,
    /// Interrupt handlers: where the prologue saves runtime registers, and
    /// which ones (decided once the body is lowered).
    runtime_save_at: usize,
    saved_runtime: Vec<(&'static str, u16)>,
}

/// Base-page runtime registers an interrupt handler may need to preserve,
/// with their sizes. `__m65_fp` is handled by frame setup itself.
const INTERRUPT_SAVED_RUNTIME: [(&str, u16); 5] = [
    (ACC_SYMBOL, 4),
    (PTR_SYMBOL, 2),
    (FARP_SYMBOL, 4),
    (ARG_SYMBOL, 4),
    (TMP_SYMBOL, TMP_BYTES),
];

/// Where one byte of a value lives.
enum ByteSource {
    Operand(Operand),
    /// Frame byte at this (already remapped) `fp` offset.
    Frame(u16),
    A,
    X,
    Y,
}

impl<'a> RoutineLowerer<'a> {
    fn new(routine: &'a Routine) -> Self {
        // A frame for frame locals, spilled values, or (through the frame
        // pointer) stack arguments.
        let reentrant = matches!(routine.abi, Abi::Reentrant);
        let stack_args = routine.abi.stack_arguments() && !routine.params.is_empty();
        let frame = (reentrant || stack_args || routine.stack_frame_bytes != 0)
            .then_some(routine.stack_frame_bytes);
        let param_bytes = if routine.abi.stack_arguments() {
            routine
                .params
                .iter()
                .map(|param| param.ty.size_bytes())
                .sum()
        } else {
            0
        };
        Self {
            routine,
            out: Vec::new(),
            next_label: 0,
            frame,
            param_bytes,
            pending_result: None,
            shared_epilogue_used: false,
            runtime_save_at: 0,
            saved_runtime: Vec::new(),
        }
    }

    fn unsupported(&self, what: impl Into<String>) -> LowerError {
        LowerError::Unsupported {
            routine: self.routine.name.clone(),
            what: what.into(),
        }
    }

    fn emit(&mut self, op: Op, operand: Operand) {
        self.out.push(Inst::op(op, operand));
    }

    fn implied(&mut self, op: Op) {
        self.out.push(Inst::implied(op));
    }

    fn local_label(&mut self) -> String {
        self.next_label += 1;
        format!("@m65_{}", self.next_label)
    }

    fn is_interrupt(&self) -> bool {
        matches!(self.routine.abi, Abi::Interrupt) && !self.routine.interrupt_raw
    }

    // ---- prologue / epilogue -----------------------------------------------

    fn prologue(&mut self) {
        if self.is_interrupt() {
            // The interrupted code may be mid-way through a `(bp),z` sequence,
            // so save every register and re-establish Z = 0 for the handler.
            for op in [Op::Pha, Op::Phx, Op::Phy, Op::Phz] {
                self.implied(op);
            }
            self.emit(Op::Ldz, Operand::imm_byte(0));
            self.runtime_save_at = self.out.len();
        }
        if let Some(frame) = self.frame {
            self.frame_helper(ENTER_ROUTINE, frame);
        }
    }

    fn shared_epilogue_label(&self) -> String {
        format!("__m65_return_{}", self.routine.name)
    }

    /// Makes the handler preserve the base-page runtime registers it can
    /// disturb: those its own code names, or all of them if it calls out.
    /// Inserts the saves into the prologue; the shared epilogue restores them.
    fn save_runtime_registers(&mut self) {
        // The frame routines only touch the accumulator and pointer; anything
        // else (including the math routines) may use every register.
        let calls_out = self.out.iter().any(|inst| {
            matches!(inst, Inst::Op(Op::Jsr, operand)
                if ![ENTER_ROUTINE, LEAVE_ROUTINE]
                    .iter()
                    .any(|helper| operand_mentions(operand, helper)))
        });
        let framed = self.frame.is_some();
        let used = |symbol: &str, out: &[Inst]| {
            out.iter().any(|inst| match inst {
                Inst::Op(_, operand) => operand_mentions(operand, symbol),
                Inst::Raw(line) => line.contains(symbol),
                Inst::Label(_) => false,
            })
        };
        self.saved_runtime = INTERRUPT_SAVED_RUNTIME
            .into_iter()
            .filter(|(symbol, _)| {
                calls_out
                    || used(symbol, &self.out)
                    || (framed && (*symbol == ACC_SYMBOL || *symbol == PTR_SYMBOL))
            })
            .collect();
        let mut saves = Vec::new();
        for (symbol, bytes) in &self.saved_runtime {
            for i in 0..*bytes {
                saves.push(Inst::op(Op::Lda, Operand::Base(offset(symbol, i))));
                saves.push(Inst::implied(Op::Pha));
            }
        }
        self.out
            .splice(self.runtime_save_at..self.runtime_save_at, saves);
    }

    /// Returns from the routine. Routines with a frame, and interrupt
    /// handlers, share one epilogue: releasing a frame or restoring saved
    /// state takes far more code than a 3-byte branch.
    fn emit_return(&mut self) {
        if self.frame.is_some() || self.is_interrupt() {
            self.shared_epilogue_used = true;
            let label = self.shared_epilogue_label();
            self.emit(Op::Lbra, Operand::Label(label));
        } else {
            self.epilogue();
        }
    }

    /// Calls a runtime frame routine with Z = the frame's local bytes (Z is
    /// already zero at both call sites). The frame setup is the same in every
    /// routine, so sharing it saves ~20 bytes per framed routine.
    fn frame_helper(&mut self, helper: &str, frame: u16) {
        if frame != 0 {
            self.emit(Op::Ldz, Operand::imm_byte(frame as u8));
        }
        self.emit(Op::Jsr, Operand::Label(helper.to_owned()));
    }

    /// Releases the frame and returns. Preserves A, X and Y (the result).
    fn epilogue(&mut self) {
        if let Some(frame) = self.frame {
            self.frame_helper(LEAVE_ROUTINE, frame);
        }
        if self.is_interrupt() {
            for (symbol, bytes) in self.saved_runtime.clone().into_iter().rev() {
                for i in (0..bytes).rev() {
                    self.implied(Op::Pla);
                    self.emit(Op::Sta, Operand::Base(offset(symbol, i)));
                }
            }
            for op in [Op::Plz, Op::Ply, Op::Plx, Op::Pla] {
                self.implied(op);
            }
            self.implied(Op::Rti);
        } else if matches!(self.routine.abi, Abi::Interrupt) {
            self.implied(Op::Rti);
        } else if self.param_bytes != 0 {
            self.emit(Op::Rtn, Operand::imm_byte(self.param_bytes as u8));
        } else {
            // `far` has no meaning yet on this target: everything is called
            // with JSR/RTS until MAP-based banking exists.
            self.implied(Op::Rts);
        }
    }

    /// Maps a storage-pass stack offset (65816 `offset,s` numbering) to an
    /// `fp` offset.
    fn frame_offset(&self, stack_offset: u16) -> Result<u16, LowerError> {
        let Some(frame) = self.frame else {
            return Err(self.unsupported("stack values in a routine without a frame"));
        };
        let mapped = if stack_offset > frame {
            stack_offset + SAVED_FP_BYTES
        } else {
            stack_offset
        };
        if mapped > 0xff {
            return Err(self.unsupported(format!("stack frame offset {mapped} beyond 255")));
        }
        Ok(mapped)
    }

    // ---- instruction lowering ----------------------------------------------

    fn lower_inst(&mut self, inst: &IrInst) -> Result<(), LowerError> {
        if self.pending_result.is_some()
            && !matches!(
                inst,
                IrInst::CleanupStack { .. }
                    | IrInst::Move {
                        src: Value::IndexX,
                        ..
                    }
            )
        {
            return Err(self.unsupported("a call result held across other instructions"));
        }
        match inst {
            IrInst::Label(label) => self.out.push(Inst::Label(label.to_string())),
            IrInst::InlineAsm(line) => self.out.push(Inst::Raw(line.clone())),
            IrInst::Move {
                width,
                dst,
                src: Value::IndexX,
            } if self.pending_result.is_some() => {
                // The frontend parks reentrant call results "in X" across the
                // caller's stack cleanup (a 65816 necessity). Here the cleanup
                // is free and the result is still in A/X/Y.
                let result_width = self.pending_result.take().unwrap_or(*width);
                self.store_result(result_width.min_bytes(*width), dst)?;
            }
            IrInst::Move { width, dst, src } => {
                if dst != src {
                    self.copy_value(*width, dst, src)?;
                }
            }
            IrInst::LoadConst { dst, value } => {
                self.copy_value(value.width(), dst, &Value::Const(*value))?;
            }
            IrInst::LoadAddress { dst, symbol, width } => {
                for i in 0..width.size_bytes() {
                    // Symbols are 24-bit; a flat address's top byte is zero.
                    let operand = match i {
                        0 => Operand::Immediate(format!("<{symbol}")),
                        1 => Operand::Immediate(format!(">{symbol}")),
                        2 => Operand::Immediate(format!("^{symbol}")),
                        _ => Operand::imm_byte(0),
                    };
                    self.emit(Op::Lda, operand);
                    self.store_a_to_byte(dst, i)?;
                }
            }
            IrInst::Load { width, dst, addr } => self.load(*width, dst, addr)?,
            IrInst::Store { width, addr, src } => self.store(*width, addr, src)?,
            IrInst::Add {
                width,
                dst,
                lhs,
                rhs,
            } => {
                // The IR never reads the carry of an Add, so `x = x + 1` can
                // use the memory increment forms.
                if is_step_in_place(dst, lhs, rhs) {
                    return self.inc(*width, dst);
                }
                self.implied(Op::Clc);
                self.byte_op(*width, Op::Adc, dst, lhs, rhs)?;
            }
            IrInst::Sub {
                width,
                dst,
                lhs,
                rhs,
            } => {
                if is_step_in_place(dst, lhs, rhs) {
                    return self.dec(*width, dst);
                }
                self.implied(Op::Sec);
                self.byte_op(*width, Op::Sbc, dst, lhs, rhs)?;
            }
            IrInst::BitAnd {
                width,
                dst,
                lhs,
                rhs,
            } => self.byte_op(*width, Op::And, dst, lhs, rhs)?,
            IrInst::BitOr {
                width,
                dst,
                lhs,
                rhs,
            } => self.byte_op(*width, Op::Ora, dst, lhs, rhs)?,
            IrInst::ShiftLeft {
                width,
                dst,
                lhs,
                rhs,
            } => self.shift(*width, true, false, dst, lhs, rhs)?,
            IrInst::ShiftRight {
                width,
                dst,
                lhs,
                rhs,
            } => self.shift(*width, false, false, dst, lhs, rhs)?,
            IrInst::Binary {
                op,
                width,
                dst,
                lhs,
                rhs,
            } => self.binary(*op, *width, dst, lhs, rhs)?,
            IrInst::SignExtend {
                width,
                src_width,
                dst,
                src,
            } => self.sign_extend(*width, *src_width, dst, src)?,
            IrInst::Inc { width, value } => self.inc(*width, value)?,
            IrInst::Dec { width, value } => self.dec(*width, value)?,
            IrInst::Compare {
                width,
                signed,
                lhs,
                rhs,
            } => self.compare(*width, *signed, lhs, rhs)?,
            IrInst::Branch { target } => self.branch(Op::Lbra, target),
            IrInst::BranchIfEqual { target } => self.branch(Op::Lbeq, target),
            IrInst::BranchIfNotEqual { target } => self.branch(Op::Lbne, target),
            IrInst::BranchIfCarrySet { target } => self.branch(Op::Lbcs, target),
            IrInst::BranchIfCarryClear { target } => self.branch(Op::Lbcc, target),
            IrInst::BranchIfZero { value, target } => {
                // The IR does not carry a width here; only byte values reach it today.
                self.load_byte_to_a(value, 0)?;
                self.branch(Op::Lbeq, target);
            }
            IrInst::Push { width, value } => {
                // High byte first, so the value sits little-endian in memory.
                for i in (0..width.size_bytes()).rev() {
                    self.load_byte_to_a(value, i)?;
                    self.implied(Op::Pha);
                }
            }
            // Reentrant callees pop their own arguments (`RTN #n`).
            IrInst::CleanupStack { .. } => {}
            IrInst::CallProcedure { name, .. } => {
                self.emit(Op::Jsr, Operand::Label(name.clone()));
            }
            IrInst::CallFunction {
                name, width, dst, ..
            } => {
                self.emit(Op::Jsr, Operand::Label(name.clone()));
                self.take_result(*width, dst)?;
            }
            IrInst::CallIndirect { target, result, .. } => {
                // The arguments are pushed; the vector goes in the scratch
                // pointer, which the callee's prologue is free to reuse.
                for i in 0..2 {
                    self.load_byte_to_a(target, i)?;
                    self.emit(Op::Sta, Operand::Base(offset(PTR_SYMBOL, i)));
                }
                self.emit(Op::Jsr, Operand::Indirect(PTR_SYMBOL.to_owned()));
                if let Some((width, dst)) = result {
                    self.take_result(*width, dst)?;
                }
            }
            IrInst::LoadRegisters { args } => self.load_registers(args)?,
            IrInst::MoveCarry { dst } => {
                self.emit(Op::Lda, Operand::imm_byte(0));
                self.implied(Op::Rol);
                self.store_a_to_byte(dst, 0)?;
            }
            IrInst::Return => self.emit_return(),
            IrInst::ReturnValue(value) if self.routine.result_in_carry => {
                // The result is the carry: set for any non-zero value.
                match value {
                    Value::Const(value) if value.bits() != 0 => self.implied(Op::Sec),
                    Value::Const(_) => self.implied(Op::Clc),
                    _ => {
                        self.load_byte_to_a(value, 0)?;
                        self.emit(Op::Cmp, Operand::imm_byte(1));
                    }
                }
                self.emit_return();
            }
            IrInst::ReturnValue(value) => {
                let RoutineKind::Function { result } = &self.routine.kind else {
                    return Err(self.unsupported("returning a value from a procedure"));
                };
                let width = width_for_type(result)
                    .map_err(|_| self.unsupported(format!("function result type {result:?}")))?;
                self.load_result(width, value)?;
                self.emit_return();
            }
            IrInst::Trap {
                family,
                call,
                x,
                y,
                dst,
            } => {
                // Call the family dispatcher with the reentrant ABI:
                // dispatcher(call, arg0, arg1), arguments pushed last-first.
                for value in [y, x, call] {
                    self.lower_inst(&IrInst::Push {
                        width: Width::Word,
                        value: value.clone(),
                    })?;
                }
                let vector = TRAP_TABLE + u16::from(*family) * 2;
                self.emit(Op::Jsr, Operand::Indirect(format!("${vector:04x}")));
                self.store_result(Width::Word, dst)?;
            }
            IrInst::CopSignature { .. } | IrInst::CopRegister { .. } => {
                return Err(self.unsupported("COP trap frames (65816-only)"));
            }
        }
        Ok(())
    }

    /// Loads register arguments: X and Y first (they may go through A),
    /// then A. An A byte that has to be read through Y (a frame value),
    /// while Y is wanted too, waits on the stack.
    fn load_registers(&mut self, args: &[(ArgRegister, Value)]) -> Result<(), LowerError> {
        let mut bytes: Vec<(ArgRegister, &Value, u16)> = Vec::new();
        for (register, value) in args {
            for i in 0..register.bytes() {
                bytes.push((register.byte(i), value, i));
            }
        }
        let a = bytes
            .iter()
            .find(|(target, _, _)| *target == ArgRegister::A)
            .copied();
        let wants_y = bytes.iter().any(|(target, _, _)| *target == ArgRegister::Y);
        let mut a_waits = false;
        if let Some((_, value, i)) = a {
            if wants_y && matches!(self.value_byte(value, i)?, ByteSource::Frame(_)) {
                self.load_byte_to_a(value, i)?;
                self.implied(Op::Pha);
                a_waits = true;
            }
        }
        for target in [ArgRegister::X, ArgRegister::Y] {
            for &(_, value, i) in bytes.iter().filter(|(register, _, _)| *register == target) {
                let (load, transfer) = if target == ArgRegister::X {
                    (Op::Ldx, Op::Tax)
                } else {
                    (Op::Ldy, Op::Tay)
                };
                match self.value_byte(value, i)? {
                    ByteSource::Operand(operand) => self.emit(load, operand),
                    _ => {
                        self.load_byte_to_a(value, i)?;
                        self.implied(transfer);
                    }
                }
            }
        }
        if let Some((_, value, i)) = a {
            if a_waits {
                self.implied(Op::Pla);
            } else {
                self.load_byte_to_a(value, i)?;
            }
        }
        Ok(())
    }

    /// Stores a call's result, or leaves it in the result registers for a
    /// following `Move` from X (see `IrInst::CallFunction`).
    fn take_result(&mut self, width: Width, dst: &Value) -> Result<(), LowerError> {
        if matches!(dst, Value::IndexX) {
            self.pending_result = Some(width);
            Ok(())
        } else {
            self.store_result(width, dst)
        }
    }

    fn branch(&mut self, op: Op, target: &Label) {
        self.emit(op, Operand::Label(target.to_string()));
    }

    // ---- value byte access -------------------------------------------------

    /// Describes byte `i` of `value`.
    fn value_byte(&self, value: &Value, i: u16) -> Result<ByteSource, LowerError> {
        Ok(match value {
            Value::Const(value) => ByteSource::Operand(Operand::imm_byte(value.byte(i))),
            Value::Accumulator => ByteSource::Operand(Operand::Base(offset(ACC_SYMBOL, i))),
            Value::Direct(name) => ByteSource::Operand(Operand::Base(offset(name, i))),
            Value::Temp(name) | Value::Local(name) | Value::Param(name) | Value::Global(name) => {
                ByteSource::Operand(Operand::Absolute(offset(name, i)))
            }
            Value::Stack { offset, .. } => ByteSource::Frame(self.frame_offset(offset + i)?),
            Value::Register(register) => match register.byte(i) {
                ArgRegister::A => ByteSource::A,
                ArgRegister::X => ByteSource::X,
                _ => ByteSource::Y,
            },
            Value::IndexX if i == 0 => ByteSource::X,
            Value::IndexY if i == 0 => ByteSource::Y,
            // Index registers are 8 bits: their upper bytes read as zero.
            Value::IndexX | Value::IndexY => ByteSource::Operand(Operand::imm_byte(0)),
        })
    }

    /// Emits `op` on byte `i` of `value`, setting Y first for frame bytes.
    /// `LDY` leaves A and the carry alone, so this is safe mid-arithmetic.
    fn value_op(&mut self, op: Op, value: &Value, i: u16) -> Result<(), LowerError> {
        match self.value_byte(value, i)? {
            ByteSource::Operand(operand) => self.emit(op, operand),
            ByteSource::Frame(fp_offset) => self.frame_op(op, fp_offset),
            ByteSource::A | ByteSource::X | ByteSource::Y => {
                return Err(self.unsupported("a register as an arithmetic operand"))
            }
        }
        Ok(())
    }

    fn frame_op(&mut self, op: Op, fp_offset: u16) {
        self.emit(Op::Ldy, Operand::imm_byte(fp_offset as u8));
        self.emit(op, Operand::IndirectY(FP_SYMBOL.to_owned()));
    }

    fn load_byte_to_a(&mut self, value: &Value, i: u16) -> Result<(), LowerError> {
        match self.value_byte(value, i)? {
            ByteSource::A => {}
            ByteSource::X => self.implied(Op::Txa),
            ByteSource::Y => self.implied(Op::Tya),
            _ => self.value_op(Op::Lda, value, i)?,
        }
        Ok(())
    }

    fn store_a_to_byte(&mut self, value: &Value, i: u16) -> Result<(), LowerError> {
        match value {
            Value::IndexX if i == 0 => self.implied(Op::Tax),
            Value::IndexY if i == 0 => self.implied(Op::Tay),
            Value::IndexX | Value::IndexY => {}
            Value::Const(_) => return Err(self.unsupported("storing into a constant")),
            _ => self.value_op(Op::Sta, value, i)?,
        }
        Ok(())
    }

    fn copy_value(&mut self, width: Width, dst: &Value, src: &Value) -> Result<(), LowerError> {
        for i in 0..width.size_bytes() {
            let source = self.value_byte(src, i)?;
            let target = self.value_byte(dst, i);
            match (&source, dst, target) {
                // `stz` stores the Z register, which is zero by invariant.
                (ByteSource::Operand(operand), _, Ok(ByteSource::Operand(target)))
                    if operand.is_zero_immediate()
                        && matches!(target, Operand::Absolute(_) | Operand::Base(_)) =>
                {
                    self.emit(Op::Stz, target);
                }
                (ByteSource::Operand(operand), Value::IndexX, _) if i == 0 => {
                    self.emit(Op::Ldx, operand.clone());
                }
                (ByteSource::Operand(operand), Value::IndexY, _) if i == 0 => {
                    self.emit(Op::Ldy, operand.clone());
                }
                _ => {
                    self.load_byte_to_a(src, i)?;
                    self.store_a_to_byte(dst, i)?;
                }
            }
        }
        Ok(())
    }

    fn byte_op(
        &mut self,
        width: Width,
        op: Op,
        dst: &Value,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), LowerError> {
        for i in 0..width.size_bytes() {
            self.load_byte_to_a(lhs, i)?;
            self.value_op(op, rhs, i)?;
            self.store_a_to_byte(dst, i)?;
        }
        Ok(())
    }

    /// Constant shifts: whole bytes move first, then the remaining bits
    /// ripple through the bytes in the base-page accumulator. A `signed`
    /// right shift fills with copies of the sign bit. When most of a byte
    /// is left over (`<< 7`), the bytes move one further and the bits shift
    /// back the other way, the byte that moved out feeding them in.
    fn shift(
        &mut self,
        width: Width,
        left: bool,
        signed: bool,
        dst: &Value,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), LowerError> {
        let Value::Const(count) = rhs else {
            return Err(self.unsupported("shifting by a non-constant count"));
        };
        let count = u16::from(count.byte(0));
        let bytes = width.size_bytes();
        let (mut byte_shift, mut bit_shift) = (count / 8, count % 8);
        // Shifting back costs (8 - bits) passes plus saving the dropped byte.
        let back = bit_shift > 0
            && byte_shift + 1 < bytes
            && bytes * (8 - bit_shift) + 3 < bytes * bit_shift;
        if back {
            // The byte that moves out: its bits come back in.
            let dropped = if left {
                bytes - 1 - byte_shift
            } else {
                byte_shift
            };
            self.load_byte_to_a(lhs, dropped)?;
            self.implied(Op::Pha);
            byte_shift += 1;
            bit_shift = 8 - bit_shift;
        }
        // Shift in place when the destination is compiler-owned memory (a
        // user global may be an I/O register); otherwise in the base-page
        // accumulator.
        let in_place = (0..bytes).all(|i| {
            matches!(
                self.value_byte(dst, i),
                Ok(ByteSource::Operand(Operand::Base(symbol) | Operand::Absolute(symbol)))
                    if symbol.starts_with("__")
            )
        });
        let work = if in_place {
            dst.clone()
        } else {
            Value::Accumulator
        };
        let operand = |this: &Self, i: u16| match this.value_byte(&work, i) {
            Ok(ByteSource::Operand(operand)) => operand,
            _ => unreachable!("the working value is plain memory"),
        };
        // Iterate so each source byte is read before the same location is written.
        let order: Vec<u16> = if left {
            (0..bytes).rev().collect()
        } else {
            (0..bytes).collect()
        };
        for i in order {
            let source = if left {
                i.checked_sub(byte_shift)
            } else {
                Some(i + byte_shift).filter(|source| *source < bytes)
            };
            match source {
                Some(source) if source == i && lhs == &work => {}
                Some(source) => {
                    self.load_byte_to_a(lhs, source)?;
                    self.store_a_to_byte(&work, i)?;
                }
                // Bytes run in ascending order for right shifts, so the
                // moved-down sign byte is already in place.
                None if signed => {
                    self.sign_fill_from(&work, bytes - 1 - byte_shift)?;
                    let target = operand(self, i);
                    self.emit(Op::Sta, target);
                }
                None => {
                    let target = operand(self, i);
                    self.emit(Op::Stz, target);
                }
            }
        }
        if back {
            self.implied(Op::Pla);
            for _ in 0..bit_shift {
                if left {
                    // Bits 0, 1, ... of the dropped byte enter at the top.
                    self.implied(Op::Lsr);
                    for i in (0..bytes).rev() {
                        let target = operand(self, i);
                        self.emit(Op::Ror, target);
                    }
                } else {
                    // Bits 7, 6, ... of the dropped byte enter at the bottom.
                    self.implied(Op::Asl);
                    for i in 0..bytes {
                        let target = operand(self, i);
                        self.emit(Op::Rol, target);
                    }
                }
            }
            bit_shift = 0;
        }
        // The bytes the whole-byte move filled with zeros stay zero as the
        // bits shift (the sign fill of an arithmetic shift does not), so
        // the passes leave them out.
        let (low, high) = if left {
            (byte_shift.min(bytes), bytes)
        } else if signed {
            (0, bytes)
        } else {
            (0, bytes.saturating_sub(byte_shift))
        };
        for _ in 0..bit_shift {
            if left {
                for i in low..high {
                    let op = if i == low { Op::Asl } else { Op::Rol };
                    let target = operand(self, i);
                    self.emit(op, target);
                }
            } else {
                for i in (low..high).rev() {
                    let target = operand(self, i);
                    let op = if i + 1 != high {
                        Op::Ror
                    } else if signed {
                        // Carry in a copy of the sign bit.
                        self.emit(Op::Lda, target.clone());
                        self.implied(Op::Asl);
                        Op::Ror
                    } else {
                        Op::Lsr
                    };
                    self.emit(op, target);
                }
            }
        }
        if &work != dst {
            self.copy_value(width, dst, &work)?;
        }
        Ok(())
    }

    /// Leaves A = $FF if byte `i` of `value` is negative, else $00.
    fn sign_fill_from(&mut self, value: &Value, i: u16) -> Result<(), LowerError> {
        self.load_byte_to_a(value, i)?;
        self.implied(Op::Asl);
        self.emit(Op::Lda, Operand::imm_byte(0xff));
        self.emit(Op::Adc, Operand::imm_byte(0));
        self.emit(Op::Eor, Operand::imm_byte(0xff));
        Ok(())
    }

    fn sign_extend(
        &mut self,
        width: Width,
        src_width: Width,
        dst: &Value,
        src: &Value,
    ) -> Result<(), LowerError> {
        let src_bytes = src_width.size_bytes();
        self.copy_value(src_width, dst, src)?;
        self.sign_fill_from(src, src_bytes - 1)?;
        for i in src_bytes..width.size_bytes() {
            self.store_a_to_byte(dst, i)?;
        }
        Ok(())
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        width: Width,
        dst: &Value,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), LowerError> {
        let (routine, result) = match op {
            BinaryOp::Xor => return self.byte_op(width, Op::Eor, dst, lhs, rhs),
            BinaryOp::ShiftRightSigned => return self.shift(width, false, true, dst, lhs, rhs),
            BinaryOp::Mul => return self.multiply(width, dst, lhs, rhs),
            BinaryOp::Div { signed: false } => return self.divide(width, false, dst, lhs, rhs),
            BinaryOp::Rem { signed: false } => return self.divide(width, true, dst, lhs, rhs),
            BinaryOp::Div { signed } => (div_routine(signed), ACC_SYMBOL),
            BinaryOp::Rem { signed } => (div_routine(signed), ARG_SYMBOL),
        };
        // The right operand goes first: the left may be the accumulator.
        let arg = Value::Direct(ARG_SYMBOL.to_owned());
        self.copy_value(width, &arg, rhs)?;
        self.copy_value(width, &Value::Accumulator, lhs)?;
        if routine != MUL_ROUTINE {
            self.emit(Op::Ldx, Operand::imm_byte(width.size_bytes() as u8));
        }
        self.emit(Op::Jsr, Operand::Label(routine.to_owned()));
        self.copy_value(width, dst, &Value::Direct(result.to_owned()))
    }

    /// Starts a use of the math unit. Interrupt code may use it too, so
    /// interrupts are held off until [`Self::release_math_unit`], except in
    /// an interrupt handler, where they already are.
    fn hold_math_unit(&mut self) {
        if !matches!(self.routine.abi, Abi::Interrupt) {
            self.implied(Op::Php);
            self.implied(Op::Sei);
        }
    }

    fn release_math_unit(&mut self) {
        if !matches!(self.routine.abi, Abi::Interrupt) {
            self.implied(Op::Plp);
        }
    }

    /// Waits while the math unit's busy flag is set: `bvs` for the
    /// multiplier (bit 6), `bmi` for the divider (bit 7).
    fn wait_math_unit(&mut self, busy: Op) {
        let wait = self.local_label();
        self.out.push(Inst::Label(wait.clone()));
        self.emit(Op::Bit, Operand::Absolute(MATH_BUSY.to_owned()));
        self.emit(busy, Operand::Label(wait));
    }

    /// `dst = lhs * rhs` on the hardware multiplier. The low `n` bytes of a
    /// product depend only on the low `n` bytes of its factors, so only
    /// those are written, whatever the inputs' upper bytes hold.
    fn multiply(
        &mut self,
        width: Width,
        dst: &Value,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), LowerError> {
        let (a, b, out) = math_registers();
        self.hold_math_unit();
        self.copy_value(width, &a, lhs)?;
        self.copy_value(width, &b, rhs)?;
        self.wait_math_unit(Op::Bvs);
        self.copy_value(width, dst, &out)?;
        self.release_math_unit();
        Ok(())
    }

    /// Unsigned `dst = lhs / rhs` (or, with `remainder`, `lhs % rhs`) on
    /// the hardware divider, which divides 32 bits by 32: the operands'
    /// upper bytes are zeroed. The remainder is `lhs - quotient * rhs`,
    /// the quotient multiplied back on the multiplier (whose other input
    /// still holds `rhs`). Dividing by zero gives a quotient of all ones
    /// (so a remainder of `lhs`).
    fn divide(
        &mut self,
        width: Width,
        remainder: bool,
        dst: &Value,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), LowerError> {
        let (a, b, product) = math_registers();
        let quotient = Value::Global(DIV_QUOTIENT.to_owned());
        let bytes = width.size_bytes();
        self.hold_math_unit();
        self.copy_value(width, &a, lhs)?;
        self.copy_value(width, &b, rhs)?;
        for i in bytes..4 {
            self.emit(Op::Stz, Operand::Absolute(offset(MULT_A, i)));
            self.emit(Op::Stz, Operand::Absolute(offset(MULT_B, i)));
        }
        self.wait_math_unit(Op::Bmi);
        if remainder {
            self.copy_value(width, &a, &quotient)?;
            self.wait_math_unit(Op::Bvs);
            self.implied(Op::Sec);
            self.byte_op(width, Op::Sbc, dst, lhs, &product)?;
        } else {
            self.copy_value(width, dst, &quotient)?;
        }
        self.release_math_unit();
        Ok(())
    }

    fn inc(&mut self, width: Width, value: &Value) -> Result<(), LowerError> {
        match self.value_byte(value, 0)? {
            ByteSource::X => {
                self.implied(Op::Inx);
                return Ok(());
            }
            ByteSource::Frame(_) => {
                self.implied(Op::Clc);
                return self.byte_op(width, Op::Adc, value, value, &Value::Const(Const::U8(1)));
            }
            ByteSource::Operand(Operand::Base(low)) if width == Width::Word => {
                self.emit(Op::Inw, Operand::Base(low));
                return Ok(());
            }
            _ => {}
        }
        let bytes = width.size_bytes();
        let done = self.local_label();
        for i in 0..bytes {
            self.value_op(Op::Inc, value, i)?;
            if i + 1 < bytes {
                self.emit(Op::Bne, Operand::Label(done.clone()));
            }
        }
        if bytes > 1 {
            self.out.push(Inst::Label(done));
        }
        Ok(())
    }

    fn dec(&mut self, width: Width, value: &Value) -> Result<(), LowerError> {
        match self.value_byte(value, 0)? {
            ByteSource::Frame(_) => {
                self.implied(Op::Sec);
                return self.byte_op(width, Op::Sbc, value, value, &Value::Const(Const::U8(1)));
            }
            ByteSource::Operand(Operand::Base(low)) if width == Width::Word => {
                self.emit(Op::Dew, Operand::Base(low));
                return Ok(());
            }
            ByteSource::X | ByteSource::Y => {
                return Err(self.unsupported("decrementing an index register"));
            }
            _ => {}
        }
        // Borrow chain: byte i+1 is decremented only if bytes 0..=i were all zero.
        let bytes = width.size_bytes();
        let labels = (1..bytes).map(|_| self.local_label()).collect::<Vec<_>>();
        for i in 0..bytes.saturating_sub(1) {
            self.value_op(Op::Lda, value, i)?;
            self.emit(Op::Bne, Operand::Label(labels[usize::from(i)].clone()));
        }
        for i in (0..bytes).rev() {
            self.value_op(Op::Dec, value, i)?;
            if i > 0 {
                self.out
                    .push(Inst::Label(labels[usize::from(i - 1)].clone()));
            }
        }
        Ok(())
    }

    /// Compare: leaves C and Z exactly as a single wide `cmp` would. A signed
    /// compare flips both sign bits, which maps two's-complement order onto
    /// unsigned order.
    fn compare(
        &mut self,
        width: Width,
        signed: bool,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), LowerError> {
        let bytes = width.size_bytes();
        let done = self.local_label();
        for i in (0..bytes).rev() {
            if signed && i + 1 == bytes {
                let rhs_operand = match rhs {
                    Value::Const(value) => Operand::imm_byte(value.byte(i) ^ 0x80),
                    _ => {
                        // The scratch pointer is free between instructions.
                        self.load_byte_to_a(rhs, i)?;
                        self.emit(Op::Eor, Operand::imm_byte(0x80));
                        self.emit(Op::Sta, Operand::Base(PTR_SYMBOL.to_owned()));
                        Operand::Base(PTR_SYMBOL.to_owned())
                    }
                };
                self.load_byte_to_a(lhs, i)?;
                self.emit(Op::Eor, Operand::imm_byte(0x80));
                self.emit(Op::Cmp, rhs_operand);
            } else if matches!(self.value_byte(lhs, i)?, ByteSource::X) {
                self.value_op(Op::Cpx, rhs, i)?;
            } else {
                self.load_byte_to_a(lhs, i)?;
                self.value_op(Op::Cmp, rhs, i)?;
            }
            if i > 0 {
                self.emit(Op::Bne, Operand::Label(done.clone()));
            }
        }
        if bytes > 1 {
            self.out.push(Inst::Label(done));
        }
        Ok(())
    }

    // ---- calls and returns -------------------------------------------------

    /// Puts `value` in the result registers: A (byte 0), X (byte 1), Y (byte 2)
    /// and ACC+3 (byte 3, which the frame release leaves alone).
    fn load_result(&mut self, width: Width, value: &Value) -> Result<(), LowerError> {
        let bytes = width.size_bytes();
        if bytes > 3 {
            self.load_byte_to_a(value, 3)?;
            self.emit(Op::Sta, Operand::Base(offset(ACC_SYMBOL, 3)));
        }
        // Frame loads use Y, so byte 2 is parked on the stack until A is loaded.
        if bytes > 2 {
            self.load_byte_to_a(value, 2)?;
            self.implied(Op::Pha);
        }
        if bytes > 1 {
            self.load_byte_to_a(value, 1)?;
            self.implied(Op::Tax);
        }
        self.load_byte_to_a(value, 0)?;
        if bytes > 2 {
            self.implied(Op::Ply);
        }
        Ok(())
    }

    /// Stores the A/X/Y result registers into `dst`.
    fn store_result(&mut self, width: Width, dst: &Value) -> Result<(), LowerError> {
        if matches!(dst, Value::IndexX | Value::IndexY) {
            return Err(self.unsupported("capturing a call result in an index register"));
        }
        let bytes = width.size_bytes();
        // Frame stores use Y, so byte 2 is saved before byte 0 is stored.
        if bytes > 2 {
            self.implied(Op::Phy);
        }
        self.store_a_to_byte(dst, 0)?;
        if bytes > 1 {
            self.implied(Op::Txa);
            self.store_a_to_byte(dst, 1)?;
        }
        if bytes > 2 {
            self.implied(Op::Pla);
            self.store_a_to_byte(dst, 2)?;
        }
        if bytes > 3 {
            self.emit(Op::Lda, Operand::Base(offset(ACC_SYMBOL, 3)));
            self.store_a_to_byte(dst, 3)?;
        }
        Ok(())
    }

    // ---- memory addressing -------------------------------------------------

    /// Prepares any scratch pointer needed for `addr` and returns its form,
    /// plus whether the form indexes by Z. `x_free` says the access does not
    /// otherwise involve X.
    fn address_form(
        &mut self,
        addr: &Address,
        x_free: bool,
    ) -> Result<(AddressForm, bool), LowerError> {
        Ok(match addr {
            Address::Symbol(name) => (AddressForm::Absolute(name.clone()), false),
            Address::Direct(name) => (AddressForm::Base(name.clone()), false),
            Address::SymbolIndexedX(name) => (AddressForm::AbsoluteX(name.clone()), false),
            Address::Indexed {
                base,
                index,
                byte_offset,
            } => {
                if let Value::Const(value) = index {
                    let offset_bytes = value.bits() as u16;
                    (AddressForm::Absolute(offset(base, offset_bytes)), false)
                } else if *byte_offset && x_free {
                    // The byte offset fits in X.
                    match self.value_byte(index, 0)? {
                        ByteSource::X => {}
                        ByteSource::Operand(operand) => self.emit(Op::Ldx, operand),
                        _ => {
                            self.load_byte_to_a(index, 0)?;
                            self.implied(Op::Tax);
                        }
                    }
                    (AddressForm::AbsoluteX(base.clone()), false)
                } else {
                    // X is only 8 bits, so form the full 16-bit address.
                    self.implied(Op::Clc);
                    for (i, part) in ["<", ">"].into_iter().enumerate() {
                        let i = i as u16;
                        // Parenthesised: ca65 applies `<`/`>` before `+`.
                        self.emit(Op::Lda, Operand::Immediate(format!("{part}({base})")));
                        self.value_op(Op::Adc, index, i)?;
                        self.emit(Op::Sta, Operand::Base(offset(PTR_SYMBOL, i)));
                    }
                    (AddressForm::IndirectZ(PTR_SYMBOL.to_owned(), 0), true)
                }
            }
            Address::Absolute(address) => {
                (AddressForm::Absolute(self.near_address(*address)?), false)
            }
            Address::AbsoluteIndexedX(address) => {
                (AddressForm::AbsoluteX(self.near_address(*address)?), false)
            }
            Address::ValueOffset {
                base,
                offset: field,
            } => match base {
                Value::Direct(name) => (AddressForm::Base(offset(name, *field)), false),
                Value::Temp(name)
                | Value::Local(name)
                | Value::Param(name)
                | Value::Global(name) => (AddressForm::Absolute(offset(name, *field)), false),
                Value::Stack {
                    offset: stack_offset,
                    ..
                } => (AddressForm::Frame(stack_offset + field), false),
                _ => return Err(self.unsupported(format!("record base {base:?}"))),
            },
            Address::NearPtr(pointer) => {
                let base = self.pointer_in_base_page(pointer)?;
                (AddressForm::IndirectZ(base, 0), true)
            }
            Address::FarPtr(pointer) => {
                let base = self.far_pointer_in_base_page(pointer)?;
                (AddressForm::FlatZ(base, 0), true)
            }
            // The offset goes in Z: no copy of the pointer plus offset.
            Address::PointerOffset {
                pointer,
                far: false,
                offset: start,
            } => {
                let base = self.pointer_in_base_page(pointer)?;
                (AddressForm::IndirectZ(base, *start), true)
            }
            Address::PointerOffset {
                pointer,
                far: true,
                offset: start,
            } => {
                let base = self.far_pointer_in_base_page(pointer)?;
                (AddressForm::FlatZ(base, *start), true)
            }
        })
    }

    fn near_address(&self, address: u32) -> Result<String, LowerError> {
        if address > 0xffff {
            return Err(self.unsupported(format!(
                "absolute address ${address:06x} outside the 16-bit CPU space (use a far pointer)"
            )));
        }
        Ok(format!("${address:04x}"))
    }

    /// Returns a base-page symbol holding the near pointer `pointer`, copying
    /// it into the scratch pointer when it lives elsewhere.
    fn pointer_in_base_page(&mut self, pointer: &Value) -> Result<String, LowerError> {
        if let Value::Direct(name) = pointer {
            return Ok(name.clone());
        }
        for i in 0..2 {
            self.load_byte_to_a(pointer, i)?;
            self.emit(Op::Sta, Operand::Base(offset(PTR_SYMBOL, i)));
        }
        Ok(PTR_SYMBOL.to_owned())
    }

    /// Returns a base-page symbol holding the far pointer `pointer`, copying
    /// it into the flat pointer when it lives elsewhere.
    fn far_pointer_in_base_page(&mut self, pointer: &Value) -> Result<String, LowerError> {
        if let Value::Direct(name) = pointer {
            return Ok(name.clone());
        }
        for i in 0..4 {
            self.load_byte_to_a(pointer, i)?;
            self.emit(Op::Sta, Operand::Base(offset(FARP_SYMBOL, i)));
        }
        Ok(FARP_SYMBOL.to_owned())
    }

    /// Emits `op` (LDA or STA) on byte `i` of an address form.
    fn address_op(&mut self, op: Op, form: &AddressForm, i: u16) -> Result<(), LowerError> {
        match form {
            AddressForm::Absolute(name) => self.emit(op, Operand::Absolute(offset(name, i))),
            AddressForm::AbsoluteX(name) => self.emit(op, Operand::AbsoluteX(offset(name, i))),
            AddressForm::Base(name) => self.emit(op, Operand::Base(offset(name, i))),
            AddressForm::Frame(stack_offset) => {
                let fp_offset = self.frame_offset(stack_offset + i)?;
                self.frame_op(op, fp_offset);
            }
            AddressForm::IndirectZ(name, start) => {
                let z = u16::from(*start) + i;
                if z > 0 {
                    self.emit(Op::Ldz, Operand::imm_byte(z as u8));
                }
                self.emit(op, Operand::IndirectZ(name.clone()));
            }
            AddressForm::FlatZ(name, start) => {
                let z = u16::from(*start) + i;
                if z > 0 {
                    self.emit(Op::Ldz, Operand::imm_byte(z as u8));
                }
                self.emit(op, Operand::FlatZ(name.clone()));
            }
        }
        Ok(())
    }

    fn load(&mut self, width: Width, dst: &Value, addr: &Address) -> Result<(), LowerError> {
        let (form, uses_z) = self.address_form(addr, !matches!(dst, Value::IndexX))?;
        let bytes = width.size_bytes();
        for i in 0..bytes {
            self.address_op(Op::Lda, &form, i)?;
            self.store_a_to_byte(dst, i)?;
        }
        if uses_z && (bytes > 1 || form.z_start() > 0) {
            self.emit(Op::Ldz, Operand::imm_byte(0));
        }
        Ok(())
    }

    fn store(&mut self, width: Width, addr: &Address, src: &Value) -> Result<(), LowerError> {
        let (form, uses_z) = self.address_form(addr, !matches!(src, Value::IndexX))?;
        let bytes = width.size_bytes();
        for i in 0..bytes {
            self.load_byte_to_a(src, i)?;
            self.address_op(Op::Sta, &form, i)?;
        }
        if uses_z && (bytes > 1 || form.z_start() > 0) {
            self.emit(Op::Ldz, Operand::imm_byte(0));
        }
        Ok(())
    }
}

enum AddressForm {
    Absolute(String),
    AbsoluteX(String),
    Base(String),
    /// Frame bytes, by storage-pass stack offset (remapped when emitted).
    Frame(u16),
    /// Through a base-page pointer, by Z from a starting offset.
    IndirectZ(String, u8),
    FlatZ(String, u8),
}

impl AddressForm {
    /// Where Z starts for byte 0 of a Z-indexed form.
    fn z_start(&self) -> u8 {
        match self {
            AddressForm::IndirectZ(_, start) | AddressForm::FlatZ(_, start) => *start,
            _ => 0,
        }
    }
}

trait WidthExt {
    fn min_bytes(self, other: Width) -> Width;
}

impl WidthExt for Width {
    fn min_bytes(self, other: Width) -> Width {
        if self.size_bytes() <= other.size_bytes() {
            self
        } else {
            other
        }
    }
}

fn operand_mentions(operand: &Operand, symbol: &str) -> bool {
    match operand {
        Operand::Implied => false,
        Operand::Immediate(text)
        | Operand::Absolute(text)
        | Operand::AbsoluteX(text)
        | Operand::Base(text)
        | Operand::IndirectZ(text)
        | Operand::IndirectY(text)
        | Operand::FlatZ(text)
        | Operand::Label(text)
        | Operand::Indirect(text) => text == symbol || text.starts_with(&format!("{symbol}+")),
    }
}

/// `dst = dst ± 1` on a memory value (not an index register).
fn is_step_in_place(dst: &Value, lhs: &Value, rhs: &Value) -> bool {
    let one = matches!(rhs, Value::Const(value) if value.bits() == 1);
    one && dst == lhs
        && matches!(
            dst,
            Value::Accumulator
                | Value::Direct(_)
                | Value::Temp(_)
                | Value::Local(_)
                | Value::Param(_)
                | Value::Global(_)
                | Value::Stack { .. }
        )
}

/// The math unit (MEGA65 I/O): the multiplier's (and divider's) inputs,
/// the product, the quotient's whole part, and the busy flags.
const MULT_A: &str = "$d770";
const MULT_B: &str = "$d774";
const MULT_OUT: &str = "$d778";
const DIV_QUOTIENT: &str = "$d76c";
const MATH_BUSY: &str = "$d70f";

fn math_registers() -> (Value, Value, Value) {
    (
        Value::Global(MULT_A.to_owned()),
        Value::Global(MULT_B.to_owned()),
        Value::Global(MULT_OUT.to_owned()),
    )
}

fn div_routine(signed: bool) -> &'static str {
    if signed {
        DIVS_ROUTINE
    } else {
        DIVU_ROUTINE
    }
}

/// `name + i`, folding into an existing numeric offset (`sym+3` + 1 is
/// `sym+4`), so the optimizer can recognize the byte.
fn offset(name: &str, i: u16) -> String {
    if i == 0 {
        return name.to_owned();
    }
    if let Some((base, existing)) = name.rsplit_once('+') {
        if let Ok(existing) = existing.parse::<u16>() {
            return format!("{base}+{}", existing + i);
        }
    }
    format!("{name}+{i}")
}

fn width_for_type(ty: &Type) -> Result<Width, ()> {
    Width::from_bytes(ty.size_bytes()).ok_or(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    Unsupported { routine: String, what: String },
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { routine, what } => write!(
                f,
                "45GS02 backend does not support {what} yet (in routine `{routine}`)"
            ),
        }
    }
}

impl std::error::Error for LowerError {}

#[cfg(test)]
mod tests {
    use crate::driver::{compile_source, Target, DEFAULT_DIRECT_PAGE_BUDGET};

    const FIELDS: &str = "
        module Fields
        type Pair = record(a: u8, b: u16, c: u8)
        export function read_b(p: near ^Pair): u16
            return p^.b
        end
        export procedure write_c(p: far ^Pair, v: u8)
            p^.c = v
            return
        end
        ";

    fn routine<'a>(asm: &'a str, name: &str) -> &'a str {
        let start = asm
            .find(&format!(".proc {name}\n"))
            .expect("routine emitted");
        let end = asm[start..].find(".endproc").expect("routine ends") + start;
        &asm[start..end]
    }

    #[test]
    fn constant_offsets_through_pointers_start_z_at_the_offset() {
        let asm = compile_source(FIELDS, Target::Mega65, DEFAULT_DIRECT_PAGE_BUDGET)
            .unwrap()
            .asm;
        // The near pointer is read in place, from Z = 1 and 2; Z ends at 0.
        let read = routine(&asm, "read_b");
        assert!(read.contains("lda (__dp_read_b_param_p),z"), "{read}");
        assert!(
            !read.contains("adc") && !read.contains("__m65_ptr"),
            "{read}"
        );
        assert!(read.contains("ldz #$00"), "{read}");
        // The far pointer stays in its base-page parameter; Z = 3 for `c`.
        let write = routine(&asm, "write_c");
        assert!(
            write.contains("ldz #$03\n\tsta [__dp_write_c_param_p],z"),
            "{write}"
        );
        assert!(
            !write.contains("__m65_farp") && !write.contains("adc"),
            "{write}"
        );
    }

    #[test]
    fn names_the_assembler_would_misread_are_renamed() {
        let source = "
            module Names
            var q: u16
            var row: u8
            const BIT: [2]u8 = [1, 2]
            export function lda(): u8
                return BIT[1]
            end
            export procedure main()
                q = 1
                row = lda()
                return
            end
        ";
        for target in [Target::Mega65, Target::W65816] {
            let asm = compile_source(source, target, DEFAULT_DIRECT_PAGE_BUDGET)
                .unwrap()
                .asm;
            for name in ["q", "row", "BIT", "lda"] {
                assert!(asm.contains(&format!("_m_{name}")), "{target:?} {name}:\n{asm}");
            }
            assert!(!asm.contains("a:q\n") && !asm.contains(" row\n"), "{target:?}:\n{asm}");
        }
    }

    #[test]
    fn four_byte_copies_and_adds_use_the_quad_forms() {
        let source = "
            module Quads
            var first: u32
            var second: u32
            var total: u32
            export procedure add_and_copy()
                total = first + second
                second = total
                return
            end
        ";
        let asm = compile_source(source, Target::Mega65, DEFAULT_DIRECT_PAGE_BUDGET)
            .unwrap()
            .asm;
        let body = routine(&asm, "add_and_copy");
        assert!(
            body.contains("clc\n\tldq a:first\n\tadcq a:second\n\tstq a:total"),
            "{body}"
        );
        assert!(body.contains("stq a:second"), "{body}");
        // Z holds a byte of the value afterwards: it is zeroed again.
        assert!(body.contains("ldz #$00"), "{body}");
    }

    #[test]
    fn other_targets_get_pointer_offsets_as_computed_pointers() {
        // Near only: the 65816 has no 32-bit arithmetic for far pointers yet.
        let near = FIELDS.split("export procedure write_c").next().unwrap();
        let asm = compile_source(near, Target::W65816, DEFAULT_DIRECT_PAGE_BUDGET)
            .unwrap()
            .asm;
        // The 65816 adds the offset to a copy of the pointer and reads
        // through the copy, as before.
        let read = routine(&asm, "read_b");
        assert!(
            read.contains("inc __dp_scratch_0\n\tlda (__dp_scratch_0)"),
            "{read}"
        );
    }
}
