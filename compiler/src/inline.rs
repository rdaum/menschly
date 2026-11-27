//! Inlining small routines.
//!
//! A call to a small leaf routine in the same module (`inlinable`) is
//! replaced by a copy of its body where that pays (`worth_inlining`): the
//! routine's only call, so nothing grows; a tiny routine, whose body is no
//! bigger than the call; or a call inside a loop, where the time goes. The
//! copy: its parameters and locals become fresh
//! locals of the caller (the caller's argument stores are pointed at the
//! parameters' new homes), its temporaries and labels are renamed apart,
//! and `return` becomes a jump to the end of the copy (a function's
//! `return v` a move of `v` into the call's destination first). A routine
//! left with no calls, no taken address and no other reference, and not
//! exported, is dropped.

use std::collections::{BTreeMap, BTreeSet};

use crate::ir::{
    param_symbol, Abi, DataItem, IrInst, Label, Local, Module, Routine, Storage, Value,
};

/// The most instructions (labels aside) a routine may have to be inlined
/// at all; inside a loop, `LOOP_LIMIT`; anywhere, `TINY`.
const LIMIT: usize = 24;
const LOOP_LIMIT: usize = 16;
const TINY: usize = 4;

fn size(routine: &Routine) -> usize {
    routine
        .body
        .iter()
        .filter(|inst| !matches!(inst, IrInst::Label(_)))
        .count()
}

/// Whether instruction `at` of `body` is inside a loop: a later branch
/// goes back to a label before it.
fn in_loop(body: &[IrInst], at: usize) -> bool {
    let before: Vec<&Label> = body[..at]
        .iter()
        .filter_map(|inst| match inst {
            IrInst::Label(label) => Some(label),
            _ => None,
        })
        .collect();
    body[at + 1..].iter().any(|inst| {
        let mut back = false;
        let _ = inst.map_labels(&mut |target| {
            back |= !matches!(inst, IrInst::Label(_)) && before.contains(&target);
            target.clone()
        });
        back
    })
}

/// Whether a call to `callee` at instruction `at` of `body` should be
/// inlined, `calls` being how many calls the module makes to it.
fn worth_inlining(callee: &Routine, calls: usize, body: &[IrInst], at: usize) -> bool {
    let size = size(callee);
    (calls == 1 && !callee.export) || size <= TINY || (size <= LOOP_LIMIT && in_loop(body, at))
}

/// Whether calls to `routine` may be replaced by its body: an ordinary
/// static routine, with no register arguments or carry result, no stack
/// frame, small, and a leaf (no calls, traps or inline assembly).
fn inlinable(routine: &Routine) -> bool {
    matches!(routine.abi, Abi::Internal | Abi::Exported)
        && !routine.result_in_carry
        && routine.stack_frame_bytes == 0
        && routine.params.iter().all(|param| param.register.is_none())
        && routine
            .locals
            .iter()
            .all(|local| matches!(local.storage, Storage::Default | Storage::Static | Storage::Direct))
        && size(routine) <= LIMIT
        && routine.body.iter().all(|inst| {
            !matches!(
                inst,
                IrInst::CallProcedure { .. }
                    | IrInst::CallFunction { .. }
                    | IrInst::CallIndirect { .. }
                    | IrInst::Trap { .. }
                    | IrInst::InlineAsm(_)
                    | IrInst::LoadRegisters { .. }
                    | IrInst::Push { .. }
                    | IrInst::CleanupStack { .. }
                    | IrInst::CopSignature { .. }
                    | IrInst::CopRegister { .. }
            )
        })
}

/// Whether calls may be inlined into `routine`: its locals live in static
/// storage or the direct page, not a stack frame the frontend has sized.
fn takes_inlining(routine: &Routine) -> bool {
    matches!(routine.abi, Abi::Internal | Abi::Exported | Abi::Interrupt)
}

/// Inlines the module's calls to its small routines, then drops those no
/// longer referenced.
pub fn inline_small_routines(module: &mut Module) {
    let callees: Vec<Routine> = module
        .routines
        .iter()
        .filter(|routine| inlinable(routine))
        .cloned()
        .collect();
    if callees.is_empty() {
        return;
    }
    // How many calls the module makes to each.
    let mut calls: BTreeMap<String, usize> = BTreeMap::new();
    for inst in module.routines.iter().flat_map(|routine| routine.body.iter()) {
        if let IrInst::CallProcedure { name, .. } | IrInst::CallFunction { name, .. } = inst {
            *calls.entry(name.clone()).or_default() += 1;
        }
    }
    let mut copies = 0usize;
    for caller in module.routines.iter_mut().filter(|routine| takes_inlining(routine)) {
        let mut body = std::mem::take(&mut caller.body);
        let mut i = 0;
        // Where the argument stores for a call to each callee can start:
        // after the last call to it, inlined or not.
        let mut since: BTreeMap<String, usize> = BTreeMap::new();
        while i < body.len() {
            let (name, result) = match &body[i] {
                IrInst::CallProcedure { name, .. } => (name.clone(), None),
                IrInst::CallFunction {
                    name, width, dst, ..
                } => (name.clone(), Some((*width, dst.clone()))),
                _ => {
                    i += 1;
                    continue;
                }
            };
            let Some(callee) = callees.iter().find(|callee| {
                callee.name == name
                    && callee.name != caller.name
                    && worth_inlining(callee, calls.get(&name).copied().unwrap_or(0), &body, i)
            }) else {
                // A call left as it is still ends the stretch whose
                // argument stores a later copy may take over.
                since.insert(name, i + 1);
                i += 1;
                continue;
            };
            copies += 1;
            let tag = format!("__i{copies}");
            // The parameters' new homes, and the caller's stores to their
            // slots (since the last call to this callee) pointed there.
            let homes: Vec<(String, Value)> = callee
                .params
                .iter()
                .map(|param| {
                    let local = format!("{}_{}{tag}", callee.name, param.name);
                    caller.locals.push(Local {
                        name: local.clone(),
                        ty: param.ty.clone(),
                        storage: Storage::Default,
                    });
                    (param_symbol(&callee.name, &param.name), Value::Local(local))
                })
                .collect();
            let start = since.get(&name).copied().unwrap_or(0);
            for inst in &mut body[start..i] {
                *inst = inst.map_values(&mut |value| match value {
                    Value::Direct(slot) => homes
                        .iter()
                        .find(|(symbol, _)| symbol == slot)
                        .map_or_else(|| value.clone(), |(_, home)| home.clone()),
                    other => other.clone(),
                });
            }
            for local in &callee.locals {
                caller.locals.push(Local {
                    name: format!("{}{tag}", local.name),
                    ty: local.ty.clone(),
                    storage: local.storage.clone(),
                });
            }
            let end = Label::new(format!("__inline{copies}_end"));
            let mut copy = Vec::with_capacity(callee.body.len() + 2);
            for inst in &callee.body {
                let inst = inst
                    .map_values(&mut |value| match value {
                        Value::Param(param) => callee
                            .params
                            .iter()
                            .position(|known| &known.name == param)
                            .map_or_else(|| value.clone(), |at| homes[at].1.clone()),
                        Value::Local(local) => Value::Local(format!("{local}{tag}")),
                        Value::Temp(temp) => Value::Temp(format!("{temp}{tag}")),
                        other => other.clone(),
                    })
                    .map_labels(&mut |label| Label::new(format!("{}{tag}", label.0)));
                match inst {
                    IrInst::Return => copy.push(IrInst::Branch {
                        target: end.clone(),
                    }),
                    IrInst::ReturnValue(value) => {
                        if let Some((width, dst)) = &result {
                            copy.push(IrInst::Move {
                                width: *width,
                                dst: dst.clone(),
                                src: value,
                            });
                        }
                        copy.push(IrInst::Branch {
                            target: end.clone(),
                        });
                    }
                    other => copy.push(other),
                }
            }
            copy.push(IrInst::Label(end));
            let length = copy.len();
            body.splice(i..=i, copy);
            i += length;
            since.insert(name, i);
        }
        caller.body = body;
    }
    drop_unreferenced(module);
}

/// Drops routines nothing refers to any more (not exported, not called,
/// no address taken, not a vector, the entry or in a table).
fn drop_unreferenced(module: &mut Module) {
    let mut referenced = BTreeSet::new();
    for routine in &module.routines {
        for inst in &routine.body {
            match inst {
                IrInst::CallProcedure { name, .. } | IrInst::CallFunction { name, .. } => {
                    referenced.insert(name.clone());
                }
                IrInst::LoadAddress { symbol, .. } => {
                    referenced.insert(symbol.clone());
                }
                // Assembly may name anything.
                IrInst::InlineAsm(text) => {
                    for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                        referenced.insert(word.to_owned());
                    }
                }
                _ => {}
            }
        }
    }
    for vector in &module.vectors {
        referenced.insert(vector.routine.clone());
    }
    if let Some(entry) = &module.entry {
        referenced.insert(entry.routine.clone());
    }
    let tables = module
        .static_data
        .iter()
        .flat_map(|data| data.items.iter())
        .chain(module.globals.iter().flat_map(|global| global.init.iter().flatten()));
    for item in tables {
        if let DataItem::Address { symbol, .. } = item {
            referenced.insert(symbol.clone());
        }
    }
    module
        .routines
        .retain(|routine| routine.export || referenced.contains(&routine.name));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calls_to(module: &Module, name: &str) -> usize {
        module
            .routines
            .iter()
            .flat_map(|routine| routine.body.iter())
            .filter(|inst| {
                matches!(inst, IrInst::CallProcedure { name: called, .. }
                    | IrInst::CallFunction { name: called, .. } if called == name)
            })
            .count()
    }

    #[test]
    fn small_leaf_routines_are_inlined_and_dropped() {
        let mut module = crate::frontend::compile_source(
            "
            module Inline
            var total: u16
            function twice(n: u16): u16
                return n + n
            end
            export procedure main()
                total = twice(3)
                total = twice(total) + 1
                return
            end
            ",
        )
        .unwrap();
        inline_small_routines(&mut module);
        assert_eq!(calls_to(&module, "twice"), 0);
        assert!(module.routines.iter().all(|routine| routine.name != "twice"));
    }

    #[test]
    fn calls_left_alone_keep_their_arguments() {
        // `scale` is too big to inline outside a loop, but inlined inside
        // one: the call before the loop must still store its argument.
        let mut module = crate::frontend::compile_source(
            "
            module Inline
            var total: u16
            var i: u8
            function scale(n: u16): u16
                var m: u16
                m = n + n
                m = m + n
                m = m + 1
                return m + n
            end
            export procedure main()
                total = scale(3)
                i = 0
                while i != 4
                    total = scale(total)
                    i = i + 1
                end
                return
            end
            ",
        )
        .unwrap();
        inline_small_routines(&mut module);
        let main = module.routines.iter().find(|routine| routine.name == "main").unwrap();
        assert_eq!(calls_to(&module, "scale"), 1, "{:#?}", main.body);
        let slot = Value::Direct(param_symbol("scale", "n"));
        let call = main
            .body
            .iter()
            .position(|inst| matches!(inst, IrInst::CallFunction { name, .. } if name == "scale"))
            .unwrap();
        assert!(
            main.body[..call].iter().any(|inst| {
                let mut stores = false;
                let _ = inst.map_values(&mut |value| {
                    stores |= *value == slot;
                    value.clone()
                });
                stores
            }),
            "{:#?}",
            main.body
        );
    }

    #[test]
    fn exported_and_calling_routines_stay() {
        let mut module = crate::frontend::compile_source(
            "
            module Inline
            var total: u16
            export function twice(n: u16): u16
                return n + n
            end
            function outer(): u16
                return twice(2)
            end
            export procedure main()
                total = outer()
                return
            end
            ",
        )
        .unwrap();
        inline_small_routines(&mut module);
        // `twice` is inlined into `outer` but, exported, still defined;
        // `outer` made a call, so it was not inlinable itself.
        assert!(module.routines.iter().any(|routine| routine.name == "twice"));
        assert_eq!(calls_to(&module, "outer"), 1);
    }
}
