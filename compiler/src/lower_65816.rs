use std::collections::BTreeMap;
use std::fmt;

use crate::ir::{
    Abi, Address, Const, CopRegister, EntryKind, GlobalPlacement, InterruptPreserve, IrInst, Label,
    Module, Routine, RoutineKind, Type, Value, Width,
};
use crate::optimize_65816::optimize_module;
use crate::storage::{apply_storage, assign_storage, StorageAssignment};
use crate::target_65816::{AsmInst, AsmModule, AsmRoutine, Immediate, Mode, Operand, Symbol};

const COP_SIGNATURE_SYMBOL: &str = "__mensch_cop_signature";
const COP_A_SYMBOL: &str = "__mensch_cop_a";
const COP_X_SYMBOL: &str = "__mensch_cop_x";
const COP_Y_SYMBOL: &str = "__mensch_cop_y";

pub fn lower_module(module: &Module) -> Result<AsmModule, LowerError> {
    Ok(optimize_module(&lower_module_unoptimized(module)?))
}

fn lower_module_unoptimized(module: &Module) -> Result<AsmModule, LowerError> {
    let mut out = AsmModule::new(module.name.clone());
    lower_module_declarations(module, &mut out);
    for routine in &module.routines {
        out.routines.push(lower_routine(routine)?);
    }
    Ok(out)
}

fn lower_module_declarations(module: &Module, out: &mut AsmModule) {
    for global in &module.globals {
        match global.placement {
            GlobalPlacement::Static => {
                out.static_symbols.push(crate::target_65816::StaticSymbol {
                    name: global.name.clone(),
                    size: global.ty.size_bytes(),
                });
            }
            GlobalPlacement::Fixed(address) => {
                out.fixed_symbols.push(crate::target_65816::FixedSymbol {
                    name: global.name.clone(),
                    address,
                    size: global.ty.size_bytes(),
                    export: global.export,
                });
            }
            GlobalPlacement::Bank(bank) => {
                out.banked_symbols.push(crate::target_65816::BankedSymbol {
                    name: global.name.clone(),
                    bank,
                    size: global.ty.size_bytes(),
                    export: global.export,
                });
            }
        }
    }
    if module.routines.iter().any(|routine| routine.interrupt_cop) {
        out.static_symbols.push(crate::target_65816::StaticSymbol {
            name: COP_SIGNATURE_SYMBOL.to_owned(),
            size: 1,
        });
        for name in [COP_A_SYMBOL, COP_X_SYMBOL, COP_Y_SYMBOL] {
            out.static_symbols.push(crate::target_65816::StaticSymbol {
                name: name.to_owned(),
                size: 2,
            });
        }
    }
    out.static_data = module
        .static_data
        .iter()
        .map(|data| crate::target_65816::StaticData {
            name: data.name.clone(),
            bytes: data.bytes.clone(),
        })
        .collect();

    out.vectors = module
        .vectors
        .iter()
        .map(|vector| crate::target_65816::VectorEntry {
            name: vector.name.clone(),
            routine: vector.routine.clone(),
        })
        .collect();
    out.metadata = module
        .metadata
        .iter()
        .map(|metadata| crate::target_65816::ModuleMetadata {
            key: metadata.key.clone(),
            value: metadata.value.clone(),
        })
        .collect();
    out.imports = module
        .imports
        .iter()
        .map(|import| crate::target_65816::ModuleImport {
            module: import.module.clone(),
            alias: import.alias.clone(),
        })
        .collect();
    out.libraries = module
        .libraries
        .iter()
        .map(|library| crate::target_65816::ModuleLibrary {
            name: library.name.clone(),
        })
        .collect();
    out.external_routines = module.external_routines.clone();
    out.external_direct_symbols = module.external_direct_symbols.clone();
    out.entry = module
        .entry
        .as_ref()
        .map(|entry| crate::target_65816::ModuleEntry {
            kind: match entry.kind {
                EntryKind::Application => "application",
                EntryKind::Command => "command",
            }
            .to_owned(),
            routine: entry.routine.clone(),
        });
}

pub fn lower_module_with_storage(
    module: &Module,
) -> Result<(AsmModule, StorageAssignment), LowerError> {
    let assignment = assign_storage(module);
    let assigned = apply_storage(module, &assignment);
    let mut lowered = lower_module_unoptimized(&assigned)?;
    let routine_exports = assigned
        .routines
        .iter()
        .map(|routine| (routine.name.clone(), routine.export))
        .collect::<BTreeMap<_, _>>();
    let direct_symbols = assignment
        .routines
        .iter()
        .flat_map(|routine| {
            let routine_export = routine_exports
                .get(&routine.name)
                .copied()
                .unwrap_or_default();
            routine
                .direct_slots
                .iter()
                .map(move |slot| (routine_export, slot))
        })
        .fold(
            BTreeMap::<String, (u16, bool)>::new(),
            |mut symbols, (routine_export, slot)| {
                let export =
                    routine_export && matches!(slot.source, crate::storage::BindingKey::Param(_));
                symbols
                    .entry(slot.symbol.clone())
                    .and_modify(|(size, existing_export)| {
                        *size = (*size).max(slot.size);
                        *existing_export |= export;
                    })
                    .or_insert((slot.size, export));
                symbols
            },
        );
    lowered.direct_symbols = direct_symbols
        .into_iter()
        .map(|(name, (size, export))| crate::target_65816::DirectSymbol { name, size, export })
        .collect();
    let static_symbols = assignment
        .routines
        .iter()
        .flat_map(|routine| routine.static_locals.iter())
        .fold(BTreeMap::<String, u16>::new(), |mut symbols, local| {
            symbols
                .entry(local.symbol.clone())
                .and_modify(|size| *size = (*size).max(local.size))
                .or_insert(local.size);
            symbols
        });
    lowered.static_symbols.extend(
        static_symbols
            .into_iter()
            .map(|(name, size)| crate::target_65816::StaticSymbol { name, size }),
    );
    Ok((optimize_module(&lowered), assignment))
}

fn lower_routine(routine: &Routine) -> Result<AsmRoutine, LowerError> {
    let mut out = AsmRoutine::new(routine.name.clone());
    out.export = routine.export;

    if !routine.interrupt_raw {
        emit_interrupt_prologue(&mut out.insts, routine.interrupt_preserve);
    }
    if matches!(routine.abi, Abi::Interrupt) && !routine.interrupt_raw {
        out.insts.push(AsmInst::Phk);
        out.insts.push(AsmInst::Plb);
        out.insts.push(AsmInst::Mode(Mode::Acc8Index16));
    }
    if routine.interrupt_cop && !routine.interrupt_raw {
        emit_cop_signature_capture(&mut out.insts, routine);
    }
    emit_stack_frame_adjust(
        &mut out.insts,
        routine.stack_frame_bytes,
        StackFrameOp::Allocate,
    );

    for inst in &routine.body {
        lower_inst(routine, inst, &mut out.insts)?;
    }

    Ok(out)
}

fn lower_inst(routine: &Routine, inst: &IrInst, out: &mut Vec<AsmInst>) -> Result<(), LowerError> {
    match inst {
        IrInst::Label(label) => out.push(AsmInst::Label(label.to_string())),
        IrInst::InlineAsm(line) => out.push(AsmInst::Raw(line.clone())),
        IrInst::Move { width, dst, src } => {
            set_mode_for_width(out, *width);
            load_value_to_a_as_width(out, src, *width)?;
            if matches!(dst, Value::IndexX) {
                out.push(AsmInst::Tax);
            } else {
                store_a_to_value(out, dst)?;
            }
        }
        IrInst::LoadConst { dst, value } => {
            set_mode_for_const(out, *value);
            match dst {
                Value::Accumulator => out.push(AsmInst::Lda(Operand::Immediate(immediate(*value)))),
                Value::IndexX => out.push(AsmInst::Ldx(Operand::Immediate(immediate(*value)))),
                Value::IndexY => out.push(AsmInst::Ldy(Operand::Immediate(immediate(*value)))),
                Value::Direct(name) => {
                    out.push(AsmInst::Lda(Operand::Immediate(immediate(*value))));
                    out.push(AsmInst::Sta(Operand::Direct(Symbol::Named(name.clone()))));
                }
                Value::Stack { .. } => {
                    out.push(AsmInst::Lda(Operand::Immediate(immediate(*value))));
                    store_a_to_value(out, dst)?;
                }
                Value::Temp(name) | Value::Local(name) | Value::Param(name) => {
                    out.push(AsmInst::Lda(Operand::Immediate(immediate(*value))));
                    out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(name.clone()))));
                }
                Value::Global(name) => {
                    out.push(AsmInst::Lda(Operand::Immediate(immediate(*value))));
                    out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(name.clone()))));
                }
                other => return Err(LowerError::UnsupportedValue(other.clone())),
            }
        }
        IrInst::LoadAddress { dst, symbol, width } => {
            set_mode_for_width(out, *width);
            out.push(AsmInst::Lda(Operand::ImmediateSymbol(Symbol::Named(
                symbol.clone(),
            ))));
            store_a_to_value(out, dst)?;
        }
        IrInst::Load { width, dst, addr } => {
            set_mode_for_width(out, *width);
            let operand = load_operand_for_address(out, addr)?;
            match dst {
                Value::Accumulator => out.push(AsmInst::Lda(operand)),
                Value::IndexX => out.push(AsmInst::Ldx(operand)),
                Value::IndexY => out.push(AsmInst::Ldy(operand)),
                Value::Direct(_)
                | Value::Stack { .. }
                | Value::Temp(_)
                | Value::Local(_)
                | Value::Param(_)
                | Value::Global(_) => {
                    out.push(AsmInst::Lda(operand));
                    store_a_to_value(out, dst)?;
                }
                other => return Err(LowerError::UnsupportedValue(other.clone())),
            }
        }
        IrInst::Store { width, addr, src } => {
            set_mode_for_width(out, *width);
            if !matches!(src, Value::Accumulator) {
                load_value_to_a(out, src)?;
            }
            let operand = store_operand_for_address(out, addr)?;
            out.push(AsmInst::Sta(operand));
        }
        IrInst::Add {
            width,
            dst,
            lhs,
            rhs,
        } => {
            set_mode_for_width(out, *width);
            load_value_to_a(out, lhs)?;
            out.push(AsmInst::Clc);
            out.push(AsmInst::Adc(operand_for_value(rhs)?));
            store_a_to_value(out, dst)?;
        }
        IrInst::Sub {
            width,
            dst,
            lhs,
            rhs,
        } => {
            set_mode_for_width(out, *width);
            load_value_to_a(out, lhs)?;
            out.push(AsmInst::Sec);
            out.push(AsmInst::Sbc(operand_for_value(rhs)?));
            store_a_to_value(out, dst)?;
        }
        IrInst::BitAnd {
            width,
            dst,
            lhs,
            rhs,
        } => {
            set_mode_for_width(out, *width);
            load_value_to_a(out, lhs)?;
            out.push(AsmInst::And(operand_for_value(rhs)?));
            store_a_to_value(out, dst)?;
        }
        IrInst::Dec { width, value } => {
            set_mode_for_width(out, *width);
            match value {
                Value::Direct(name) => {
                    out.push(AsmInst::Dec(Operand::Direct(Symbol::Named(name.clone()))))
                }
                Value::Temp(name)
                | Value::Local(name)
                | Value::Param(name)
                | Value::Global(name) => {
                    out.push(AsmInst::Dec(Operand::Absolute(Symbol::Named(name.clone()))))
                }
                other => return Err(LowerError::UnsupportedValue(other.clone())),
            }
        }
        IrInst::Inc { width, value } => {
            set_mode_for_width(out, *width);
            match value {
                Value::IndexX => out.push(AsmInst::Inx),
                Value::Direct(name) => {
                    out.push(AsmInst::Inc(Operand::Direct(Symbol::Named(name.clone()))))
                }
                Value::Temp(name)
                | Value::Local(name)
                | Value::Param(name)
                | Value::Global(name) => {
                    out.push(AsmInst::Inc(Operand::Absolute(Symbol::Named(name.clone()))))
                }
                other => return Err(LowerError::UnsupportedValue(other.clone())),
            }
        }
        IrInst::Compare { width, lhs, rhs } => {
            set_mode_for_width(out, *width);
            match lhs {
                Value::Accumulator => out.push(AsmInst::Cmp(operand_for_value(rhs)?)),
                Value::IndexX => out.push(AsmInst::Cpx(operand_for_value(rhs)?)),
                other => {
                    load_value_to_a(out, other)?;
                    out.push(AsmInst::Cmp(operand_for_value(rhs)?));
                }
            }
        }
        IrInst::Branch { target } => out.push(AsmInst::Brl(target.to_string())),
        IrInst::BranchIfZero { value, target } => {
            load_value_to_a(out, value)?;
            out.push(long_branch_if_equal(target));
        }
        IrInst::BranchIfEqual { target } => out.push(long_branch_if_equal(target)),
        IrInst::BranchIfNotEqual { target } => out.push(long_branch_if_not_equal(target)),
        IrInst::BranchIfCarrySet { target } => out.push(long_branch_if_carry_set(target)),
        IrInst::BranchIfCarryClear { target } => out.push(long_branch_if_carry_clear(target)),
        IrInst::Push { width, value } => {
            set_mode_for_width(out, *width);
            load_value_to_a(out, value)?;
            out.push(AsmInst::Pha);
        }
        IrInst::CallProcedure { name } => out.push(AsmInst::Jsr(name.clone())),
        IrInst::CallFunction { name, dst } => {
            out.push(AsmInst::Jsr(name.clone()));
            store_a_to_value(out, dst)?;
        }
        IrInst::CleanupStack { bytes } => {
            if *bytes != 0 {
                out.push(AsmInst::Mode(Mode::Acc16Index16));
                out.push(AsmInst::Tsc);
                out.push(AsmInst::Clc);
                out.push(AsmInst::Adc(Operand::Immediate(Immediate::Word(*bytes))));
                out.push(AsmInst::Tcs);
            }
        }
        IrInst::CopSignature { dst } => {
            set_mode_for_width(out, Width::Byte);
            out.push(AsmInst::Lda(Operand::Absolute(Symbol::Named(
                COP_SIGNATURE_SYMBOL.to_owned(),
            ))));
            store_a_to_value(out, dst)?;
        }
        IrInst::CopRegister { register, dst } => {
            set_mode_for_width(out, Width::Word);
            out.push(AsmInst::Lda(Operand::Absolute(Symbol::Named(
                cop_register_symbol(*register).to_owned(),
            ))));
            store_a_to_value(out, dst)?;
        }
        IrInst::CopCall {
            family,
            call,
            x,
            y,
            dst,
        } => {
            set_mode_for_width(out, Width::Word);
            load_value_to_a(out, y)?;
            out.push(AsmInst::Tay);
            load_value_to_a(out, x)?;
            out.push(AsmInst::Tax);
            load_value_to_a(out, call)?;
            out.push(AsmInst::Raw(format!("cop ${family:02x}")));
            store_a_to_value(out, dst)?;
        }
        IrInst::Return => {
            emit_return_sequence(routine, out);
        }
        IrInst::ReturnValue(value) => {
            if let RoutineKind::Function { result } = &routine.kind {
                let width = width_for_type(result)?;
                set_mode_for_width(out, width);
                if !matches!(value, Value::Accumulator) {
                    load_value_to_a(out, value)?;
                }
                if routine.stack_frame_bytes != 0 && matches!(width, Width::Byte | Width::Word) {
                    out.push(AsmInst::Tax);
                    emit_stack_frame_adjust(out, routine.stack_frame_bytes, StackFrameOp::Release);
                    set_mode_for_width(out, width);
                    out.push(AsmInst::Txa);
                    if !routine.interrupt_raw {
                        emit_interrupt_epilogue(out, routine.interrupt_preserve);
                    }
                    out.push(return_inst(routine));
                    return Ok(());
                }
            }
            emit_return_sequence(routine, out);
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackFrameOp {
    Allocate,
    Release,
}

fn emit_stack_frame_adjust(out: &mut Vec<AsmInst>, bytes: u16, op: StackFrameOp) {
    if bytes == 0 {
        return;
    }

    out.push(AsmInst::Mode(Mode::Acc16Index16));
    out.push(AsmInst::Tsc);
    match op {
        StackFrameOp::Allocate => {
            out.push(AsmInst::Sec);
            out.push(AsmInst::Sbc(Operand::Immediate(Immediate::Word(bytes))));
        }
        StackFrameOp::Release => {
            out.push(AsmInst::Clc);
            out.push(AsmInst::Adc(Operand::Immediate(Immediate::Word(bytes))));
        }
    }
    out.push(AsmInst::Tcs);
}

fn emit_cop_signature_capture(out: &mut Vec<AsmInst>, routine: &Routine) {
    let pcl_offset = interrupt_frame_pcl_offset(routine.interrupt_preserve);
    let pbr_offset = pcl_offset + 2;
    let load_label = format!("__{}_cop_signature_load", routine.name);

    out.push(AsmInst::Mode(Mode::Acc16Index16));
    out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(
        COP_A_SYMBOL.to_owned(),
    ))));
    out.push(AsmInst::Stx(Operand::Absolute(Symbol::Named(
        COP_X_SYMBOL.to_owned(),
    ))));
    out.push(AsmInst::Sty(Operand::Absolute(Symbol::Named(
        COP_Y_SYMBOL.to_owned(),
    ))));
    out.push(AsmInst::Lda(
        stack_operand(pcl_offset).expect("COP frame offset fits in one byte"),
    ));
    out.push(AsmInst::Raw("dec a".to_owned()));
    out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(format!(
        "{load_label}+1"
    )))));
    out.push(AsmInst::Mode(Mode::Acc8Index16));
    out.push(AsmInst::Lda(
        stack_operand(pbr_offset).expect("COP frame offset fits in one byte"),
    ));
    out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(format!(
        "{load_label}+3"
    )))));
    out.push(AsmInst::Label(load_label));
    out.push(AsmInst::Raw("lda f:$000000".to_owned()));
    out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(
        COP_SIGNATURE_SYMBOL.to_owned(),
    ))));
    out.push(AsmInst::Mode(Mode::Acc16Index16));
}

fn interrupt_frame_pcl_offset(preserve: InterruptPreserve) -> u16 {
    2 + interrupt_preserve_bytes(preserve)
}

fn interrupt_preserve_bytes(preserve: InterruptPreserve) -> u16 {
    u16::from(preserve.p)
        + u16::from(preserve.dbr)
        + if preserve.a { 2 } else { 0 }
        + if preserve.x { 2 } else { 0 }
        + if preserve.y { 2 } else { 0 }
}

fn cop_register_symbol(register: CopRegister) -> &'static str {
    match register {
        CopRegister::A => COP_A_SYMBOL,
        CopRegister::X => COP_X_SYMBOL,
        CopRegister::Y => COP_Y_SYMBOL,
    }
}

fn return_inst(routine: &Routine) -> AsmInst {
    match routine.abi {
        Abi::Exported | Abi::Far => AsmInst::Rtl,
        Abi::Interrupt => AsmInst::Rti,
        Abi::Internal | Abi::Reentrant => match routine.kind {
            RoutineKind::Procedure | RoutineKind::Function { .. } => AsmInst::Rts,
        },
    }
}

fn long_branch_if_equal(target: &Label) -> AsmInst {
    AsmInst::Raw(format!("bne :+\n\tbrl {}\n:", target))
}

fn long_branch_if_not_equal(target: &Label) -> AsmInst {
    AsmInst::Raw(format!("beq :+\n\tbrl {}\n:", target))
}

fn long_branch_if_carry_set(target: &Label) -> AsmInst {
    AsmInst::Raw(format!("bcc :+\n\tbrl {}\n:", target))
}

fn long_branch_if_carry_clear(target: &Label) -> AsmInst {
    AsmInst::Raw(format!("bcs :+\n\tbrl {}\n:", target))
}

fn emit_return_sequence(routine: &Routine, out: &mut Vec<AsmInst>) {
    emit_stack_frame_adjust(out, routine.stack_frame_bytes, StackFrameOp::Release);
    if !routine.interrupt_raw {
        emit_interrupt_epilogue(out, routine.interrupt_preserve);
    }
    out.push(return_inst(routine));
}

fn emit_interrupt_prologue(out: &mut Vec<AsmInst>, preserve: InterruptPreserve) {
    if preserve.is_empty() {
        return;
    }
    if preserve.p {
        out.push(AsmInst::Php);
    }
    if preserve.dbr {
        out.push(AsmInst::Phb);
    }
    if preserve.a || preserve.x || preserve.y {
        out.push(AsmInst::Mode(Mode::Acc16Index16));
    }
    if preserve.a {
        out.push(AsmInst::Pha);
    }
    if preserve.x {
        out.push(AsmInst::Phx);
    }
    if preserve.y {
        out.push(AsmInst::Phy);
    }
}

fn emit_interrupt_epilogue(out: &mut Vec<AsmInst>, preserve: InterruptPreserve) {
    if preserve.is_empty() {
        return;
    }
    if preserve.a || preserve.x || preserve.y {
        out.push(AsmInst::Mode(Mode::Acc16Index16));
    }
    if preserve.y {
        out.push(AsmInst::Ply);
    }
    if preserve.x {
        out.push(AsmInst::Plx);
    }
    if preserve.a {
        out.push(AsmInst::Pla);
    }
    if preserve.dbr {
        out.push(AsmInst::Plb);
    }
    if preserve.p {
        out.push(AsmInst::Plp);
    }
}

fn set_mode_for_const(out: &mut Vec<AsmInst>, value: Const) {
    match value {
        Const::U8(_) => out.push(AsmInst::Mode(Mode::Acc8Index16)),
        Const::U16(_) | Const::U24(_) => out.push(AsmInst::Mode(Mode::Acc16Index16)),
    }
}

fn set_mode_for_width(out: &mut Vec<AsmInst>, width: Width) {
    match width {
        Width::Byte => out.push(AsmInst::Mode(Mode::Acc8Index16)),
        Width::Word | Width::Long => out.push(AsmInst::Mode(Mode::Acc16Index16)),
    }
}

fn width_for_type(ty: &Type) -> Result<Width, LowerError> {
    width_for_size(ty.size_bytes())
}

fn width_for_size(size: u16) -> Result<Width, LowerError> {
    match size {
        1 => Ok(Width::Byte),
        2 => Ok(Width::Word),
        3 => Ok(Width::Long),
        _ => Err(LowerError::UnsupportedWidth(size)),
    }
}

fn immediate(value: Const) -> Immediate {
    match value {
        Const::U8(value) => Immediate::Byte(value),
        Const::U16(value) => Immediate::Word(value),
        Const::U24(value) => Immediate::Long(value),
    }
}

fn operand_for_address(addr: &Address) -> Result<Operand, LowerError> {
    match addr {
        Address::Symbol(name) => Ok(Operand::Absolute(Symbol::Named(name.clone()))),
        Address::SymbolIndexedX(name) => Ok(Operand::AbsoluteIndexedX(Symbol::Named(name.clone()))),
        Address::Direct(name) => Ok(Operand::Direct(Symbol::Named(name.clone()))),
        Address::ValueOffset { base, offset } => operand_for_value_offset(base, *offset),
        Address::Absolute(addr) => Ok(Operand::Absolute(Symbol::Address(*addr))),
        Address::AbsoluteIndexedX(addr) => Ok(Operand::AbsoluteIndexedX(Symbol::Address(*addr))),
        Address::NearPtr(value) => match value {
            Value::Direct(name) => Ok(Operand::IndirectDirect(Symbol::Named(name.clone()))),
            other => Err(LowerError::UnsupportedValue(other.clone())),
        },
        Address::FarPtr(value) => match value {
            Value::Direct(name) => Ok(Operand::IndirectLongDirect(Symbol::Named(name.clone()))),
            other => Err(LowerError::UnsupportedValue(other.clone())),
        },
    }
}

fn operand_for_value_offset(value: &Value, offset: u16) -> Result<Operand, LowerError> {
    match value {
        Value::Direct(name) => Ok(Operand::Direct(symbol_with_offset(name, offset))),
        Value::Stack {
            offset: stack_offset,
            ..
        } => stack_operand(
            stack_offset
                .checked_add(offset)
                .ok_or(LowerError::UnsupportedStackOffset(*stack_offset))?,
        ),
        Value::Temp(name) | Value::Local(name) | Value::Param(name) | Value::Global(name) => {
            Ok(Operand::Absolute(symbol_with_offset(name, offset)))
        }
        other => Err(LowerError::UnsupportedValue(other.clone())),
    }
}

fn symbol_with_offset(name: &str, offset: u16) -> Symbol {
    if offset == 0 {
        Symbol::Named(name.to_owned())
    } else {
        Symbol::Named(format!("{name}+{offset}"))
    }
}

fn load_operand_for_address(
    _out: &mut Vec<AsmInst>,
    addr: &Address,
) -> Result<Operand, LowerError> {
    match addr {
        Address::NearPtr(Value::Direct(name)) => {
            Ok(Operand::IndirectDirect(Symbol::Named(name.clone())))
        }
        other => operand_for_address(other),
    }
}

fn store_operand_for_address(
    _out: &mut Vec<AsmInst>,
    addr: &Address,
) -> Result<Operand, LowerError> {
    match addr {
        Address::NearPtr(Value::Direct(name)) => {
            Ok(Operand::IndirectDirect(Symbol::Named(name.clone())))
        }
        other => operand_for_address(other),
    }
}

fn operand_for_value(value: &Value) -> Result<Operand, LowerError> {
    match value {
        Value::Const(value) => Ok(Operand::Immediate(immediate(*value))),
        Value::Direct(name) => Ok(Operand::Direct(Symbol::Named(name.clone()))),
        Value::Stack { offset, .. } => stack_operand(*offset),
        Value::Temp(name) | Value::Local(name) | Value::Param(name) => {
            Ok(Operand::Absolute(Symbol::Named(name.clone())))
        }
        Value::Global(name) => Ok(Operand::Absolute(Symbol::Named(name.clone()))),
        other => Err(LowerError::UnsupportedValue(other.clone())),
    }
}

fn load_value_to_a(out: &mut Vec<AsmInst>, value: &Value) -> Result<(), LowerError> {
    match value {
        Value::Accumulator => Ok(()),
        Value::IndexX => {
            out.push(AsmInst::Txa);
            Ok(())
        }
        Value::IndexY => {
            out.push(AsmInst::Tya);
            Ok(())
        }
        Value::Const(value) => {
            out.push(AsmInst::Lda(Operand::Immediate(immediate(*value))));
            Ok(())
        }
        Value::Direct(name) => {
            out.push(AsmInst::Lda(Operand::Direct(Symbol::Named(name.clone()))));
            Ok(())
        }
        Value::Stack { offset, size } => {
            set_mode_for_width(out, width_for_size(*size)?);
            out.push(AsmInst::Lda(stack_operand(*offset)?));
            Ok(())
        }
        Value::Temp(name) | Value::Local(name) | Value::Param(name) => {
            out.push(AsmInst::Lda(Operand::Absolute(Symbol::Named(name.clone()))));
            Ok(())
        }
        Value::Global(name) => {
            out.push(AsmInst::Lda(Operand::Absolute(Symbol::Named(name.clone()))));
            Ok(())
        }
    }
}

fn load_value_to_a_as_width(
    out: &mut Vec<AsmInst>,
    value: &Value,
    width: Width,
) -> Result<(), LowerError> {
    set_mode_for_width(out, width);
    match value {
        Value::Accumulator => Ok(()),
        Value::IndexX => {
            out.push(AsmInst::Txa);
            Ok(())
        }
        Value::IndexY => {
            out.push(AsmInst::Tya);
            Ok(())
        }
        Value::Const(value) => {
            out.push(AsmInst::Lda(Operand::Immediate(immediate(*value))));
            Ok(())
        }
        Value::Direct(name) => {
            out.push(AsmInst::Lda(Operand::Direct(Symbol::Named(name.clone()))));
            Ok(())
        }
        Value::Stack { offset, .. } => {
            out.push(AsmInst::Lda(stack_operand(*offset)?));
            Ok(())
        }
        Value::Temp(name) | Value::Local(name) | Value::Param(name) => {
            out.push(AsmInst::Lda(Operand::Absolute(Symbol::Named(name.clone()))));
            Ok(())
        }
        Value::Global(name) => {
            out.push(AsmInst::Lda(Operand::Absolute(Symbol::Named(name.clone()))));
            Ok(())
        }
    }
}

fn store_a_to_value(out: &mut Vec<AsmInst>, value: &Value) -> Result<(), LowerError> {
    match value {
        Value::Accumulator => Ok(()),
        Value::IndexX => {
            out.push(AsmInst::Tax);
            Ok(())
        }
        Value::IndexY => {
            out.push(AsmInst::Tay);
            Ok(())
        }
        Value::Direct(name) => {
            out.push(AsmInst::Sta(Operand::Direct(Symbol::Named(name.clone()))));
            Ok(())
        }
        Value::Stack { offset, size } => {
            set_mode_for_width(out, width_for_size(*size)?);
            out.push(AsmInst::Sta(stack_operand(*offset)?));
            Ok(())
        }
        Value::Temp(name) | Value::Local(name) | Value::Param(name) => {
            out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(name.clone()))));
            Ok(())
        }
        Value::Global(name) => {
            out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(name.clone()))));
            Ok(())
        }
        other => Err(LowerError::UnsupportedValue(other.clone())),
    }
}

fn stack_operand(offset: u16) -> Result<Operand, LowerError> {
    let offset = u8::try_from(offset).map_err(|_| LowerError::UnsupportedStackOffset(offset))?;
    Ok(Operand::StackRelative(offset))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    UnsupportedValue(Value),
    UnsupportedWidth(u16),
    UnsupportedStackOffset(u16),
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedValue(value) => {
                write!(f, "unsupported value in 65816 lowering: {value:?}")
            }
            Self::UnsupportedWidth(width) => {
                write!(f, "unsupported width in 65816 lowering: {width}")
            }
            Self::UnsupportedStackOffset(offset) => {
                write!(f, "unsupported stack offset in 65816 lowering: {offset}")
            }
        }
    }
}

impl std::error::Error for LowerError {}

#[cfg(test)]
mod tests {
    use crate::ir::{Address, IrInst, Module, Routine, Storage, Type, Value, Width};
    use crate::target_65816::{AsmInst, Operand, Symbol};

    use super::lower_module;

    #[test]
    fn lowers_near_pointer_load_through_direct_slot() {
        let module = Module::new("near_ptr").with_routine(
            Routine::procedure("near_ptr")
                .with_local(crate::ir::Local::new(
                    "ptr",
                    Type::near_ptr(Type::U8),
                    Storage::Direct,
                ))
                .with_body(vec![
                    IrInst::Load {
                        width: Width::Byte,
                        dst: Value::Accumulator,
                        addr: Address::NearPtr(Value::Direct("ptr".to_owned())),
                    },
                    IrInst::Return,
                ]),
        );

        let lowered = lower_module(&module).unwrap();

        assert!(lowered.routines[0]
            .insts
            .contains(&AsmInst::Lda(Operand::IndirectDirect(Symbol::Named(
                "ptr".to_owned()
            )))));
    }

    #[test]
    fn lowers_near_pointer_store_through_direct_slot_without_zeroing_y() {
        let module = Module::new("near_ptr_store").with_routine(
            Routine::procedure("near_ptr_store")
                .with_local(crate::ir::Local::new(
                    "ptr",
                    Type::near_ptr(Type::U8),
                    Storage::Direct,
                ))
                .with_body(vec![
                    IrInst::Store {
                        width: Width::Byte,
                        addr: Address::NearPtr(Value::Direct("ptr".to_owned())),
                        src: Value::Accumulator,
                    },
                    IrInst::Return,
                ]),
        );

        let lowered = lower_module(&module).unwrap();

        assert!(lowered.routines[0]
            .insts
            .contains(&AsmInst::Sta(Operand::IndirectDirect(Symbol::Named(
                "ptr".to_owned()
            )))));
    }

    #[test]
    fn lowers_far_pointer_load_through_direct_slot() {
        let module = Module::new("far_ptr").with_routine(
            Routine::procedure("far_ptr")
                .with_local(crate::ir::Local::new(
                    "ptr",
                    Type::far_ptr(Type::U8),
                    Storage::Direct,
                ))
                .with_body(vec![
                    IrInst::Load {
                        width: Width::Byte,
                        dst: Value::Accumulator,
                        addr: Address::FarPtr(Value::Direct("ptr".to_owned())),
                    },
                    IrInst::Return,
                ]),
        );

        let lowered = lower_module(&module).unwrap();

        assert!(lowered.routines[0]
            .insts
            .contains(&AsmInst::Lda(Operand::IndirectLongDirect(Symbol::Named(
                "ptr".to_owned()
            )))));
    }
}
