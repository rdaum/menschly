//! Forwarding copies into temporaries.
//!
//! The frontend often moves a value into a temporary only for the next
//! instruction to read it (`pair.low = u16(dest)` is a move of `dest`'s
//! low word into a temporary, then a store of the temporary). When a
//! temporary is defined once, by a move, and read only by the very next
//! instruction, that instruction reads the move's source instead and the
//! move goes. Nothing runs between the two, so the source still holds the
//! value (a `volatile` one is still read once, in the same place); the
//! reader takes as many low bytes as it read of the temporary, which are
//! the source's own.

use std::collections::BTreeMap;

use crate::ir::{IrInst, Module, Routine, Value, Width};

/// How many times each temporary is mentioned in `body`.
fn mentions(body: &[IrInst]) -> BTreeMap<String, usize> {
    let mut count = BTreeMap::new();
    for inst in body {
        let _ = inst.map_values(&mut |value| {
            if let Value::Temp(name) = value {
                *count.entry(name.clone()).or_default() += 1;
            }
            value.clone()
        });
    }
    count
}

/// Whether `value` can stand in for a temporary as an operand.
fn forwardable(value: &Value) -> bool {
    matches!(
        value,
        Value::Const(_)
            | Value::Temp(_)
            | Value::Local(_)
            | Value::Param(_)
            | Value::Global(_)
            | Value::Direct(_)
    )
}

/// The width at which `inst` reads `temp`, when it reads it once, only as
/// an operand (never writes it), at a width its kind states.
fn read_width(inst: &IrInst, temp: &Value) -> Option<Width> {
    let mut times = 0;
    let _ = inst.map_values(&mut |value| {
        times += usize::from(value == temp);
        value.clone()
    });
    if times != 1 {
        return None;
    }
    match inst {
        IrInst::Move { width, src, .. } | IrInst::Store { width, src, .. } if src == temp => {
            Some(*width)
        }
        IrInst::Add { width, lhs, rhs, .. }
        | IrInst::Sub { width, lhs, rhs, .. }
        | IrInst::BitAnd { width, lhs, rhs, .. }
        | IrInst::BitOr { width, lhs, rhs, .. }
        | IrInst::Compare { width, lhs, rhs, .. }
        | IrInst::Binary { width, lhs, rhs, .. }
            if lhs == temp || rhs == temp =>
        {
            Some(*width)
        }
        IrInst::ShiftLeft { width, lhs, .. } | IrInst::ShiftRight { width, lhs, .. }
            if lhs == temp =>
        {
            Some(*width)
        }
        IrInst::SignExtend { src_width, src, .. } if src == temp => Some(*src_width),
        IrInst::Push { width, value } if value == temp => Some(*width),
        _ => None,
    }
}

fn forward_routine(routine: &mut Routine) {
    let count = mentions(&routine.body);
    let body = std::mem::take(&mut routine.body);
    let mut out: Vec<IrInst> = Vec::with_capacity(body.len());
    let mut pending: Option<(Value, Width, Value)> = None;
    for inst in body {
        let inst = match pending.take() {
            Some((temp, width, src))
                if read_width(&inst, &temp).is_some_and(|read| read.size_bytes() <= width.size_bytes()) =>
            {
                // The move is no longer needed.
                out.pop();
                inst.map_values(&mut |value| {
                    if *value == temp {
                        src.clone()
                    } else {
                        value.clone()
                    }
                })
            }
            _ => inst,
        };
        if let IrInst::Move {
            width,
            dst: dst @ Value::Temp(temp),
            src,
        } = &inst
        {
            // Mentioned twice in all: this definition and one read.
            if count.get(temp) == Some(&2) && forwardable(src) && src != dst {
                pending = Some((dst.clone(), *width, src.clone()));
            }
        }
        out.push(inst);
    }
    routine.body = out;
}

/// Forwards copies in every routine of the module.
pub fn forward_module(module: &mut Module) {
    for routine in &mut module.routines {
        forward_routine(routine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_read_by_the_next_instruction_is_forwarded() {
        let mut module = crate::frontend::compile_source(
            "
            module Forward
            type Pair = record(low: u16, top: u8)
            var pair: Pair
            var spare: u16
            export procedure f(dest: u32)
                pair.low = u16(dest)
                spare = u16(dest) + 1
                return
            end
            ",
        )
        .unwrap();
        forward_module(&mut module);
        let body = &module.routines[0].body;
        assert!(
            body.iter().any(|inst| matches!(inst,
                IrInst::Store { src: Value::Param(name), .. } if name == "dest")),
            "{body:#?}"
        );
        assert!(
            !body.iter().any(|inst| matches!(inst,
                IrInst::Move { dst: Value::Temp(_), src: Value::Param(_), .. })),
            "{body:#?}"
        );
    }
}
