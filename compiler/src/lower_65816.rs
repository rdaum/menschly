use std::collections::BTreeMap;
use std::fmt;

use crate::ir::{
    Abi, Address, Const, EntryKind, GlobalPlacement, IrInst, Module, Routine, RoutineKind, Value,
    Width,
};
use crate::optimize_65816::optimize_module;
use crate::storage::{apply_storage, assign_storage, StorageAssignment};
use crate::target_65816::{AsmInst, AsmModule, AsmRoutine, Immediate, Mode, Operand, Symbol};

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
    let direct_symbols = assignment
        .routines
        .iter()
        .flat_map(|routine| routine.direct_slots.iter())
        .fold(BTreeMap::<String, u16>::new(), |mut symbols, slot| {
            symbols
                .entry(slot.symbol.clone())
                .and_modify(|size| *size = (*size).max(slot.size))
                .or_insert(slot.size);
            symbols
        });
    lowered.direct_symbols = direct_symbols
        .into_iter()
        .map(|(name, size)| crate::target_65816::DirectSymbol { name, size })
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
    if matches!(routine.abi, Abi::Exported) {
        out.export = true;
    }

    for inst in &routine.body {
        lower_inst(routine, inst, &mut out.insts)?;
    }

    Ok(out)
}

fn lower_inst(routine: &Routine, inst: &IrInst, out: &mut Vec<AsmInst>) -> Result<(), LowerError> {
    match inst {
        IrInst::Label(label) => out.push(AsmInst::Label(label.to_string())),
        IrInst::InlineAsm(line) => out.push(AsmInst::Raw(line.clone())),
        IrInst::Move { dst, src } => {
            load_value_to_a(out, src)?;
            store_a_to_value(out, dst)?;
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
                Value::Global(name) => {
                    out.push(AsmInst::Lda(Operand::Immediate(immediate(*value))));
                    out.push(AsmInst::Sta(Operand::Absolute(Symbol::Named(name.clone()))));
                }
                other => return Err(LowerError::UnsupportedValue(other.clone())),
            }
        }
        IrInst::Load { width, dst, addr } => {
            set_mode_for_width(out, *width);
            let operand = operand_for_address(addr)?;
            match dst {
                Value::Accumulator => out.push(AsmInst::Lda(operand)),
                Value::IndexX => out.push(AsmInst::Ldx(operand)),
                Value::IndexY => out.push(AsmInst::Ldy(operand)),
                Value::Direct(_)
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
            out.push(AsmInst::Sta(operand_for_address(addr)?));
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
                other => return Err(LowerError::UnsupportedValue(other.clone())),
            }
        }
        IrInst::Compare { width, lhs, rhs } => {
            set_mode_for_width(out, *width);
            match lhs {
                Value::Accumulator => out.push(AsmInst::Cmp(operand_for_value(rhs)?)),
                Value::IndexX => out.push(AsmInst::Cpx(operand_for_value(rhs)?)),
                other => return Err(LowerError::UnsupportedValue(other.clone())),
            }
        }
        IrInst::Branch { target } => out.push(AsmInst::Bra(target.to_string())),
        IrInst::BranchIfZero { value, target } => {
            load_value_to_a(out, value)?;
            out.push(AsmInst::Beq(target.to_string()));
        }
        IrInst::BranchIfEqual { target } => out.push(AsmInst::Beq(target.to_string())),
        IrInst::BranchIfNotEqual { target } => out.push(AsmInst::Bne(target.to_string())),
        IrInst::CallProcedure { name } => out.push(AsmInst::Jsr(name.clone())),
        IrInst::CallFunction { name, dst } => {
            out.push(AsmInst::Jsr(name.clone()));
            store_a_to_value(out, dst)?;
        }
        IrInst::Return => out.push(return_inst(routine)),
        IrInst::ReturnValue(value) => {
            if !matches!(value, Value::Accumulator) {
                load_value_to_a(out, value)?;
            }
            out.push(return_inst(routine));
        }
    }

    Ok(())
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

fn operand_for_value(value: &Value) -> Result<Operand, LowerError> {
    match value {
        Value::Const(value) => Ok(Operand::Immediate(immediate(*value))),
        Value::Direct(name) => Ok(Operand::Direct(Symbol::Named(name.clone()))),
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
        Value::Const(value) => {
            out.push(AsmInst::Lda(Operand::Immediate(immediate(*value))));
            Ok(())
        }
        Value::Direct(name) => {
            out.push(AsmInst::Lda(Operand::Direct(Symbol::Named(name.clone()))));
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
        other => Err(LowerError::UnsupportedValue(other.clone())),
    }
}

fn store_a_to_value(out: &mut Vec<AsmInst>, value: &Value) -> Result<(), LowerError> {
    match value {
        Value::Accumulator => Ok(()),
        Value::Direct(name) => {
            out.push(AsmInst::Sta(Operand::Direct(Symbol::Named(name.clone()))));
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    UnsupportedValue(Value),
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedValue(value) => {
                write!(f, "unsupported value in 65816 lowering: {value:?}")
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
