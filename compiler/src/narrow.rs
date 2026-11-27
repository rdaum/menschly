//! Narrowing: computing only the bytes of a temporary that are used.
//!
//! A backward pass finds how many low bytes of each temporary are read
//! (`demands`): a move of width `w` reads `w`; an add, subtract, AND, OR,
//! XOR or left shift reads as many bytes of its operands as its result
//! needs (the low bytes of the answer depend only on the low bytes of
//! the operands); a logical right shift by `c` of which `k` bytes are
//! needed reads `(c + 8k + 7) / 8`; anything else reads a temporary whole.
//! Then each such instruction defining a temporary is done at the
//! narrowest width that covers what is needed: `u8(x >> 20)` of a 32-bit
//! `x` shifts two bytes, not four.

use std::collections::BTreeMap;

use crate::ir::{BinaryOp, Const, IrInst, Module, Routine, Value, Width};

fn width_of(bytes: u16) -> Width {
    match bytes {
        0 | 1 => Width::Byte,
        2 => Width::Word,
        3 => Width::Long,
        _ => Width::Quad,
    }
}

fn temp(value: &Value) -> Option<&str> {
    match value {
        Value::Temp(name) => Some(name),
        _ => None,
    }
}

/// The bit count of a constant shift.
fn shift_count(rhs: &Value) -> Option<u16> {
    match rhs {
        Value::Const(count) => Some(u16::from(count.byte(0))),
        _ => None,
    }
}

/// The instruction's result width, and the operands whose low bytes alone
/// make the low bytes of the result (with, for a right shift, its count).
fn narrowable(inst: &IrInst) -> Option<(Width, &Value, Vec<&Value>, Option<u16>)> {
    match inst {
        IrInst::Move { width, dst, src } => Some((*width, dst, vec![src], None)),
        IrInst::Add { width, dst, lhs, rhs }
        | IrInst::Sub { width, dst, lhs, rhs }
        | IrInst::BitAnd { width, dst, lhs, rhs }
        | IrInst::BitOr { width, dst, lhs, rhs }
        | IrInst::ShiftLeft { width, dst, lhs, rhs }
        | IrInst::Binary {
            op: BinaryOp::Xor,
            width,
            dst,
            lhs,
            rhs,
        } => Some((*width, dst, vec![lhs, rhs], None)),
        IrInst::ShiftRight { width, dst, lhs, rhs } => {
            Some((*width, dst, vec![lhs], Some(shift_count(rhs)?)))
        }
        _ => None,
    }
}

/// How many low bytes of each temporary the routine reads.
fn demands(routine: &Routine) -> BTreeMap<String, u16> {
    // A temporary's full size: the widest instruction defining it.
    let mut full: BTreeMap<String, u16> = BTreeMap::new();
    for inst in &routine.body {
        let dst = match inst {
            IrInst::LoadConst { dst, value } => Some((dst, value.width())),
            _ => narrowable(inst).map(|(width, dst, _, _)| (dst, width)),
        };
        if let Some((dst, width)) = dst {
            if let Some(name) = temp(dst) {
                let entry = full.entry(name.to_owned()).or_default();
                *entry = (*entry).max(width.size_bytes());
            }
        }
    }
    let mut demand: BTreeMap<String, u16> = BTreeMap::new();
    loop {
        let mut changed = false;
        let mut need = |name: &str, bytes: u16, demand: &mut BTreeMap<String, u16>| {
            let entry = demand.entry(name.to_owned()).or_default();
            if bytes > *entry {
                *entry = bytes;
                changed = true;
            }
        };
        for inst in routine.body.iter().rev() {
            if let Some((width, dst, operands, count)) = narrowable(inst) {
                let wanted = match temp(dst) {
                    Some(name) => demand.get(name).copied().unwrap_or(0),
                    None => width.size_bytes(),
                }
                .min(width.size_bytes());
                let reads = match count {
                    Some(count) => ((count + 8 * wanted + 7) / 8).min(width.size_bytes()),
                    None => wanted,
                };
                for operand in operands {
                    if let Some(name) = temp(operand) {
                        need(name, reads, &mut demand);
                    }
                }
                continue;
            }
            // Anything else reads every temporary it mentions, whole.
            let _ = inst.map_values(&mut |value| {
                if let Some(name) = temp(value) {
                    let bytes = full.get(name).copied().unwrap_or(4);
                    need(name, bytes, &mut demand);
                }
                value.clone()
            });
        }
        if !changed {
            return demand;
        }
    }
}

/// Narrows the routine's instructions that define temporaries.
fn narrow_routine(routine: &mut Routine) {
    let demand = demands(routine);
    for inst in &mut routine.body {
        let needed = |dst: &Value| {
            temp(dst).map(|name| demand.get(name).copied().unwrap_or(0).max(1))
        };
        match inst {
            IrInst::LoadConst { dst, value } => {
                if let Some(bytes) = needed(dst) {
                    if bytes < value.width().size_bytes() {
                        let bits = value.bits();
                        *value = match width_of(bytes) {
                            Width::Byte => Const::U8(bits as u8),
                            Width::Word => Const::U16(bits as u16),
                            Width::Long => Const::U24(bits & 0xff_ffff),
                            Width::Quad => *value,
                        };
                    }
                }
            }
            IrInst::Move { width, dst, .. }
            | IrInst::Add { width, dst, .. }
            | IrInst::Sub { width, dst, .. }
            | IrInst::BitAnd { width, dst, .. }
            | IrInst::BitOr { width, dst, .. }
            | IrInst::ShiftLeft { width, dst, .. }
            | IrInst::Binary {
                op: BinaryOp::Xor,
                width,
                dst,
                ..
            } => {
                if let Some(bytes) = needed(dst) {
                    if bytes < width.size_bytes() {
                        *width = width_of(bytes);
                    }
                }
            }
            IrInst::ShiftRight { width, dst, rhs, .. } => {
                if let (Some(bytes), Some(count)) = (needed(dst), shift_count(rhs)) {
                    let reads = (count + 8 * bytes + 7) / 8;
                    if reads < width.size_bytes() {
                        *width = width_of(reads);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Narrows every routine of the module.
pub fn narrow_module(module: &mut Module) {
    for routine in &mut module.routines {
        narrow_routine(routine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(source: &str) -> Vec<(String, Width)> {
        let mut module = crate::frontend::compile_source(source).unwrap();
        narrow_module(&mut module);
        module.routines[0]
            .body
            .iter()
            .filter_map(|inst| match inst {
                IrInst::ShiftRight { width, .. } => Some(("shift right".to_owned(), *width)),
                IrInst::Add { width, .. } => Some(("add".to_owned(), *width)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn truncated_results_are_computed_narrow() {
        // Only the low byte of the 32-bit sum is kept: a 1-byte add. Of
        // `source >> 8`, one byte is kept, which bytes 1 and 2 make: the
        // shift is done on two bytes rather than four.
        let found = widths(
            "
            module Narrow
            var source: u32
            var step: u32
            var low: u8
            var top: u8
            export procedure f()
                low = u8(source + step)
                top = u8(source >> 8)
                return
            end
            ",
        );
        assert!(found.contains(&("add".to_owned(), Width::Byte)), "{found:?}");
        assert!(found.contains(&("shift right".to_owned(), Width::Word)), "{found:?}");
    }
}
