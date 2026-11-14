//! Lowering from Mensch IR to 45GS02 (MEGA65) assembly.
//!
//! The 45GS02 accumulator is always 8 bits wide, so every IR operation is
//! lowered byte by byte: a value of width N is N bytes at some location, and
//! add/sub/compare/move walk those bytes. The IR's virtual accumulator
//! (`Value::Accumulator`) lives in the base-page runtime register
//! [`ACC_SYMBOL`]; function results come back in `A` (byte 0), `X` (byte 1)
//! and `Y` (byte 2).
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
//! COP-style traps and absolute addresses above `$FFFF`.

use std::fmt;

use crate::ir::{
    Abi, Address, Const, IrInst, Label, Module, Routine, RoutineKind, Type, Value, Width,
};
use crate::optimize_45gs02::optimize_routine;
use crate::storage::{
    apply_storage, assign_storage, assign_storage_with_budget, StorageAssignment,
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

pub const RUNTIME_DIRECT_SYMBOLS: [&str; 4] = [ACC_SYMBOL, PTR_SYMBOL, FARP_SYMBOL, FP_SYMBOL];

/// Runtime routine that builds a stack frame (Z = local bytes).
pub const ENTER_ROUTINE: &str = "__m65_enter";
/// Runtime routine that releases a stack frame (Z = local bytes).
pub const LEAVE_ROUTINE: &str = "__m65_leave";
/// Runtime routines generated code may call.
pub const RUNTIME_ROUTINES: [&str; 2] = [ENTER_ROUTINE, LEAVE_ROUTINE];

/// Fixed address of the trap vector table: family `n`'s dispatcher address
/// is the word at `TRAP_TABLE + 2n`. Part of the MEGA65 kernel ABI.
pub const TRAP_TABLE: u16 = 0xff00;

/// Bytes between the locals and the return address (the saved fp).
const SAVED_FP_BYTES: u16 = 2;

pub fn lower_module_with_storage(
    module: &Module,
) -> Result<(AsmModule, StorageAssignment), LowerError> {
    lower_with_assignment(module, assign_storage(module))
}

pub fn lower_module_with_storage_budget(
    module: &Module,
    direct_page_budget: u16,
) -> Result<(AsmModule, StorageAssignment), LowerError> {
    lower_with_assignment(
        module,
        assign_storage_with_budget(module, direct_page_budget),
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
    for routine in &assigned.routines {
        out.routines.push(lower_routine(routine)?);
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
    Ok((out, assignment))
}

fn lower_routine(routine: &Routine) -> Result<AsmRoutine, LowerError> {
    let mut lowerer = RoutineLowerer::new(routine);
    if lowerer.frame.is_some_and(|frame| frame > 0xff) {
        return Err(lowerer.unsupported(format!(
            "a {}-byte stack frame (at most 255)",
            routine.stack_frame_bytes
        )));
    }
    lowerer.prologue();
    for inst in &routine.body {
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
    out.insts = optimize_routine(lowerer.out, z_zero_on_entry);
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
const INTERRUPT_SAVED_RUNTIME: [(&str, u16); 3] =
    [(ACC_SYMBOL, 4), (PTR_SYMBOL, 2), (FARP_SYMBOL, 4)];

/// Where one byte of a value lives.
enum ByteSource {
    Operand(Operand),
    /// Frame byte at this (already remapped) `fp` offset.
    Frame(u16),
    X,
    Y,
}

impl<'a> RoutineLowerer<'a> {
    fn new(routine: &'a Routine) -> Self {
        let reentrant = matches!(routine.abi, Abi::Reentrant);
        let frame =
            (reentrant || routine.stack_frame_bytes != 0).then_some(routine.stack_frame_bytes);
        let param_bytes = if reentrant {
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
        // The frame routines only touch the accumulator and pointer.
        let calls_out = self.out.iter().any(|inst| {
            matches!(inst, Inst::Op(Op::Jsr, operand)
                if !RUNTIME_ROUTINES.iter().any(|helper| operand_mentions(operand, helper)))
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
                self.copy_value(const_width(*value), dst, &Value::Const(*value))?;
            }
            IrInst::LoadAddress { dst, symbol, width } => {
                for i in 0..width.size_bytes() {
                    let prefix = ["<", ">", "^"][usize::from(i.min(2))];
                    self.emit(Op::Lda, Operand::Immediate(format!("{prefix}{symbol}")));
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
            } => self.shift(*width, true, dst, lhs, rhs)?,
            IrInst::ShiftRight {
                width,
                dst,
                lhs,
                rhs,
            } => self.shift(*width, false, dst, lhs, rhs)?,
            IrInst::Inc { width, value } => self.inc(*width, value)?,
            IrInst::Dec { width, value } => self.dec(*width, value)?,
            IrInst::Compare { width, lhs, rhs } => self.compare(*width, lhs, rhs)?,
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
                if matches!(dst, Value::IndexX) {
                    self.pending_result = Some(*width);
                } else {
                    self.store_result(*width, dst)?;
                }
            }
            IrInst::Return => self.emit_return(),
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

    fn branch(&mut self, op: Op, target: &Label) {
        self.emit(op, Operand::Label(target.to_string()));
    }

    // ---- value byte access -------------------------------------------------

    /// Describes byte `i` of `value`.
    fn value_byte(&self, value: &Value, i: u16) -> Result<ByteSource, LowerError> {
        Ok(match value {
            Value::Const(value) => ByteSource::Operand(Operand::imm_byte(const_byte(*value, i))),
            Value::Accumulator => ByteSource::Operand(Operand::Base(offset(ACC_SYMBOL, i))),
            Value::Direct(name) => ByteSource::Operand(Operand::Base(offset(name, i))),
            Value::Temp(name) | Value::Local(name) | Value::Param(name) | Value::Global(name) => {
                ByteSource::Operand(Operand::Absolute(offset(name, i)))
            }
            Value::Stack { offset, .. } => ByteSource::Frame(self.frame_offset(offset + i)?),
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
            ByteSource::X | ByteSource::Y => {
                return Err(self.unsupported("an index register as an arithmetic operand"))
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
    /// ripple through the bytes in the base-page accumulator.
    fn shift(
        &mut self,
        width: Width,
        left: bool,
        dst: &Value,
        lhs: &Value,
        rhs: &Value,
    ) -> Result<(), LowerError> {
        let Value::Const(count) = rhs else {
            return Err(self.unsupported("shifting by a non-constant count"));
        };
        let count = u16::from(const_byte(*count, 0));
        let bytes = width.size_bytes();
        let (byte_shift, bit_shift) = (count / 8, count % 8);
        let acc = Value::Accumulator;
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
                Some(source) => {
                    self.load_byte_to_a(lhs, source)?;
                    self.store_a_to_byte(&acc, i)?;
                }
                None => self.emit(Op::Stz, Operand::Base(offset(ACC_SYMBOL, i))),
            }
        }
        for _ in 0..bit_shift {
            if left {
                for i in 0..bytes {
                    let op = if i == 0 { Op::Asl } else { Op::Rol };
                    self.emit(op, Operand::Base(offset(ACC_SYMBOL, i)));
                }
            } else {
                for i in (0..bytes).rev() {
                    let op = if i + 1 == bytes { Op::Lsr } else { Op::Ror };
                    self.emit(op, Operand::Base(offset(ACC_SYMBOL, i)));
                }
            }
        }
        if dst != &acc {
            self.copy_value(width, dst, &acc)?;
        }
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

    /// Unsigned compare: leaves C and Z exactly as a single wide `cmp` would.
    fn compare(&mut self, width: Width, lhs: &Value, rhs: &Value) -> Result<(), LowerError> {
        let bytes = width.size_bytes();
        let done = self.local_label();
        for i in (0..bytes).rev() {
            if matches!(self.value_byte(lhs, i)?, ByteSource::X) {
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

    /// Puts `value` in the result registers: A (byte 0), X (byte 1), Y (byte 2).
    fn load_result(&mut self, width: Width, value: &Value) -> Result<(), LowerError> {
        let bytes = width.size_bytes();
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
        Ok(())
    }

    // ---- memory addressing -------------------------------------------------

    /// Prepares any scratch pointer needed for `addr` and returns its form,
    /// plus whether the form indexes by Z.
    fn address_form(&mut self, addr: &Address) -> Result<(AddressForm, bool), LowerError> {
        Ok(match addr {
            Address::Symbol(name) => (AddressForm::Absolute(name.clone()), false),
            Address::Direct(name) => (AddressForm::Base(name.clone()), false),
            Address::SymbolIndexedX(name) => (AddressForm::AbsoluteX(name.clone()), false),
            Address::Indexed { base, index } => {
                if let Value::Const(value) = index {
                    let offset_bytes =
                        u16::from(const_byte(*value, 0)) | u16::from(const_byte(*value, 1)) << 8;
                    (AddressForm::Absolute(offset(base, offset_bytes)), false)
                } else {
                    // X is only 8 bits, so form the full 16-bit address.
                    self.implied(Op::Clc);
                    for (i, part) in ["<", ">"].into_iter().enumerate() {
                        let i = i as u16;
                        self.emit(Op::Lda, Operand::Immediate(format!("{part}{base}")));
                        self.value_op(Op::Adc, index, i)?;
                        self.emit(Op::Sta, Operand::Base(offset(PTR_SYMBOL, i)));
                    }
                    (AddressForm::IndirectZ(PTR_SYMBOL.to_owned()), true)
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
                (AddressForm::IndirectZ(base), true)
            }
            Address::FarPtr(pointer) => {
                // Copy the 3-byte Mensch far pointer into the 4-byte flat
                // pointer; its top byte is kept zero.
                for i in 0..3 {
                    self.load_byte_to_a(pointer, i)?;
                    self.emit(Op::Sta, Operand::Base(offset(FARP_SYMBOL, i)));
                }
                self.emit(Op::Stz, Operand::Base(offset(FARP_SYMBOL, 3)));
                (AddressForm::FlatZ(FARP_SYMBOL.to_owned()), true)
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
            AddressForm::IndirectZ(name) => {
                if i > 0 {
                    self.emit(Op::Ldz, Operand::imm_byte(i as u8));
                }
                self.emit(op, Operand::IndirectZ(name.clone()));
            }
            AddressForm::FlatZ(name) => {
                if i > 0 {
                    self.emit(Op::Ldz, Operand::imm_byte(i as u8));
                }
                self.emit(op, Operand::FlatZ(name.clone()));
            }
        }
        Ok(())
    }

    fn load(&mut self, width: Width, dst: &Value, addr: &Address) -> Result<(), LowerError> {
        let (form, uses_z) = self.address_form(addr)?;
        let bytes = width.size_bytes();
        for i in 0..bytes {
            self.address_op(Op::Lda, &form, i)?;
            self.store_a_to_byte(dst, i)?;
        }
        if uses_z && bytes > 1 {
            self.emit(Op::Ldz, Operand::imm_byte(0));
        }
        Ok(())
    }

    fn store(&mut self, width: Width, addr: &Address, src: &Value) -> Result<(), LowerError> {
        let (form, uses_z) = self.address_form(addr)?;
        let bytes = width.size_bytes();
        for i in 0..bytes {
            self.load_byte_to_a(src, i)?;
            self.address_op(Op::Sta, &form, i)?;
        }
        if uses_z && bytes > 1 {
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
    IndirectZ(String),
    FlatZ(String),
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
    let one = matches!(
        rhs,
        Value::Const(Const::U8(1)) | Value::Const(Const::U16(1)) | Value::Const(Const::U24(1))
    );
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

fn offset(name: &str, i: u16) -> String {
    if i == 0 {
        name.to_owned()
    } else {
        format!("{name}+{i}")
    }
}

fn const_width(value: Const) -> Width {
    match value {
        Const::U8(_) => Width::Byte,
        Const::U16(_) => Width::Word,
        Const::U24(_) => Width::Long,
    }
}

fn const_byte(value: Const, i: u16) -> u8 {
    let raw = match value {
        Const::U8(value) => u32::from(value),
        Const::U16(value) => u32::from(value),
        Const::U24(value) => value,
    };
    raw.checked_shr(u32::from(i) * 8).unwrap_or(0) as u8
}

fn width_for_type(ty: &Type) -> Result<Width, ()> {
    match ty.size_bytes() {
        1 => Ok(Width::Byte),
        2 => Ok(Width::Word),
        3 => Ok(Width::Long),
        _ => Err(()),
    }
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
