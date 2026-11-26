use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use crate::ir::{
    param_symbol, Abi, Address, Const, IrInst, Label, Local, Module, Param, PointerKind, Routine,
    Storage, Type, Value,
};

pub const DEFAULT_DIRECT_PAGE_BUDGET: u16 = 0x80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageAssignment {
    pub module_name: String,
    pub routines: Vec<RoutineStorage>,
    pub direct_page_bytes: u16,
    /// Private direct-page slots placed in the module's overlay region
    /// ([`OVERLAY_SYMBOL`]): symbol -> offset. Routines that can never be
    /// running at the same time share addresses there.
    pub overlay: BTreeMap<String, u16>,
    pub overlay_bytes: u16,
}

/// The module-local direct-page region private slots are overlaid in.
pub const OVERLAY_SYMBOL: &str = "__dp_overlay";

impl StorageAssignment {
    pub fn routine(&self, name: &str) -> Option<&RoutineStorage> {
        self.routines.iter().find(|routine| routine.name == name)
    }

    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("storage layout for module ");
        out.push_str(&self.module_name);
        out.push('\n');
        out.push_str(&format!("direct page bytes: {}\n", self.direct_page_bytes));

        for routine in &self.routines {
            out.push_str("\nroutine ");
            out.push_str(&routine.name);
            out.push('\n');

            for slot in &routine.direct_slots {
                out.push_str(&format!(
                    "  direct ${:02x}..${:02x} {} {:?}\n",
                    slot.offset,
                    slot.offset + slot.size - 1,
                    slot.symbol,
                    slot.source
                ));
            }

            for local in &routine.static_locals {
                out.push_str(&format!("  static {} size {}\n", local.symbol, local.size));
            }

            for slot in &routine.stack_slots {
                out.push_str(&format!(
                    "  stack +{} {} size {}\n",
                    slot.offset, slot.name, slot.size
                ));
            }
        }

        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectPageUsage {
    pub budget: u16,
    pub used: u16,
    pub modules: Vec<DirectPageModuleUsage>,
}

impl DirectPageUsage {
    pub fn remaining(&self) -> Option<u16> {
        self.budget.checked_sub(self.used)
    }

    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "direct page usage: {} / {} bytes",
            self.used, self.budget
        ));
        if let Some(remaining) = self.remaining() {
            out.push_str(&format!(" ({} free)", remaining));
        } else {
            out.push_str(&format!(" ({} over)", self.used - self.budget));
        }
        out.push('\n');
        for module in &self.modules {
            out.push_str(&format!(
                "  {}: {} bytes\n",
                module.module_name, module.bytes
            ));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectPageModuleUsage {
    pub module_name: String,
    pub bytes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectPageBudgetError {
    pub usage: DirectPageUsage,
}

impl DirectPageBudgetError {
    pub fn overage(&self) -> u16 {
        self.usage.used - self.usage.budget
    }
}

impl fmt::Display for DirectPageBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "direct page allocation exceeds budget by {} bytes",
            self.overage()
        )?;
        f.write_str(&self.usage.report())
    }
}

impl std::error::Error for DirectPageBudgetError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageAssignmentError {
    pub missing: Vec<MissingStorageBinding>,
}

impl fmt::Display for StorageAssignmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "storage assignment is incomplete")?;
        for missing in &self.missing {
            writeln!(
                f,
                "  routine {} is missing storage for {:?}",
                missing.routine, missing.key
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for StorageAssignmentError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingStorageBinding {
    pub routine: String,
    pub key: BindingKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineStorage {
    pub name: String,
    pub direct_slots: Vec<DirectSlot>,
    pub static_locals: Vec<StaticLocal>,
    pub stack_slots: Vec<StackSlot>,
    pub stack_frame_bytes: u16,
    /// Locals kept in temp registers: (symbol, bytes). They start at zero,
    /// like the stack locals they replace.
    pub zeroed_registers: Vec<(String, u16)>,
    bindings: BTreeMap<BindingKey, Location>,
}

impl RoutineStorage {
    pub fn location(&self, key: BindingKey) -> Option<&Location> {
        self.bindings.get(&key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectSlot {
    pub symbol: String,
    pub offset: u16,
    pub size: u16,
    pub source: BindingKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticLocal {
    pub name: String,
    pub symbol: String,
    pub size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackSlot {
    pub name: String,
    pub offset: u16,
    pub size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Direct {
        symbol: String,
        offset: u16,
        size: u16,
    },
    Static {
        symbol: String,
        size: u16,
    },
    Stack {
        offset: u16,
        size: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BindingKey {
    Param(String),
    Local(String),
    Temp(String),
}

pub fn direct_page_usage(assignments: &[StorageAssignment], budget: u16) -> DirectPageUsage {
    let modules = assignments
        .iter()
        .map(|assignment| DirectPageModuleUsage {
            module_name: assignment.module_name.clone(),
            bytes: assignment.direct_page_bytes,
        })
        .collect::<Vec<_>>();
    let used = modules.iter().map(|module| module.bytes).sum();
    DirectPageUsage {
        budget,
        used,
        modules,
    }
}

pub fn validate_direct_page_budget(
    assignments: &[StorageAssignment],
    budget: u16,
) -> Result<DirectPageUsage, DirectPageBudgetError> {
    let usage = direct_page_usage(assignments, budget);
    if usage.used <= budget {
        Ok(usage)
    } else {
        Err(DirectPageBudgetError { usage })
    }
}

pub fn assign_storage(module: &Module) -> StorageAssignment {
    assign_storage_with_direct_page_budget(module, None, StorageOptions::default())
}

pub fn assign_storage_with_budget(module: &Module, budget: u16) -> StorageAssignment {
    assign_storage_with_direct_page_budget(module, Some(budget), StorageOptions::default())
}

/// Target-specific storage choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageOptions {
    /// Scratch values wider than two bytes must live in the direct page
    /// (the 65816 reaches 24-bit values through `[dp]`). Targets that copy
    /// wide values into their own registers can keep them in ordinary RAM.
    pub wide_scratch_direct: bool,
    /// A block of base-page registers that interrupt handlers preserve
    /// (symbol, bytes). Reentrant routines keep expression temporaries that
    /// do not live across a call there instead of in their stack frame.
    pub temp_registers: Option<(&'static str, u16)>,
    /// Stack-frame accesses cost no more than direct-page ones (the 65816's
    /// `offset,s`), so `stack` routines put spilled scratch values in a frame
    /// rather than spend direct page on them.
    pub cheap_frames: bool,
}

impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            wide_scratch_direct: true,
            temp_registers: None,
            cheap_frames: true,
        }
    }
}

pub fn assign_storage_with_options(
    module: &Module,
    budget: Option<u16>,
    options: StorageOptions,
) -> StorageAssignment {
    assign_storage_with_direct_page_budget(module, budget, options)
}

fn assign_storage_with_direct_page_budget(
    module: &Module,
    budget: Option<u16>,
    options: StorageOptions,
) -> StorageAssignment {
    let mut allocator = DirectAllocator::new(budget);
    allocator.wide_scratch_direct = options.wide_scratch_direct;
    allocator.temp_registers = options.temp_registers;
    allocator.cheap_frames = options.cheap_frames;
    let mut args = DirectSlotPool::new("__dp_arg");
    let mut scratch = DirectSlotPool::new("__dp_scratch");
    let routines = module
        .routines
        .iter()
        .map(|routine| {
            if matches!(routine.abi, Abi::Interrupt) {
                // An interrupt can arrive while any other routine is using a
                // shared slot, so handlers get pools of their own.
                let mut private_args = DirectSlotPool::new(format!("__dp_{}_arg", routine.name));
                let mut private_scratch =
                    DirectSlotPool::new(format!("__dp_{}_scratch", routine.name));
                assign_routine(
                    routine,
                    &mut allocator,
                    &mut private_args,
                    &mut private_scratch,
                )
            } else {
                assign_routine(routine, &mut allocator, &mut args, &mut scratch)
            }
        })
        .collect();

    let mut assignment = StorageAssignment {
        module_name: module.name.clone(),
        routines,
        direct_page_bytes: allocator.next_offset,
        overlay: BTreeMap::new(),
        overlay_bytes: 0,
    };
    overlay_private_slots(module, &mut assignment);
    assignment
}

/// Overlays routines' private direct-page slots (parameters, `direct`
/// locals and temporaries that no other routine's storage shares) where the
/// routines can never be running at the same time: neither can reach the
/// other through calls. A call this module cannot follow (to an external
/// routine, through a pointer, by trap or from inline assembly) may come
/// back into any routine that is exported or has its address taken, and
/// whatever an interrupt handler can reach may run at any time, so keeps
/// slots of its own. The program's entry (the module's declared entry, or
/// else an exported `main`, which the MEGA65 runtime calls) runs once, at
/// the bottom of the stack: nothing calls back into it.
fn overlay_private_slots(module: &Module, assignment: &mut StorageAssignment) {
    let names: Vec<&str> = module
        .routines
        .iter()
        .map(|routine| routine.name.as_str())
        .collect();
    let index_of = |name: &str| names.iter().position(|known| *known == name);
    let count = names.len();
    // Direct calls, and whether a routine makes calls this module cannot follow.
    let mut calls = vec![BTreeSet::new(); count];
    let mut opaque = vec![false; count];
    let mut entered_from_outside = vec![false; count];
    for vector in &module.vectors {
        if let Some(target) = index_of(&vector.routine) {
            entered_from_outside[target] = true;
        }
    }
    for (i, routine) in module.routines.iter().enumerate() {
        if routine.export {
            entered_from_outside[i] = true;
        }
        for inst in &routine.body {
            match inst {
                IrInst::CallProcedure { name, .. } | IrInst::CallFunction { name, .. } => {
                    match index_of(name) {
                        Some(target) => {
                            calls[i].insert(target);
                        }
                        None => opaque[i] = true,
                    }
                }
                IrInst::CallIndirect { .. } | IrInst::Trap { .. } | IrInst::InlineAsm(_) => {
                    opaque[i] = true;
                }
                IrInst::LoadAddress { symbol, .. } => {
                    if let Some(target) = index_of(symbol) {
                        entered_from_outside[target] = true;
                    }
                }
                _ => {}
            }
        }
    }
    let entry = module
        .entry
        .as_ref()
        .and_then(|entry| index_of(&entry.routine))
        .or_else(|| index_of("main").filter(|&i| module.routines[i].export));
    let outside: Vec<usize> = (0..count)
        .filter(|&i| entered_from_outside[i] && Some(i) != entry)
        .collect();
    for i in 0..count {
        if opaque[i] {
            calls[i].extend(outside.iter().copied());
        }
    }
    // Transitive closure.
    let mut reach = calls.clone();
    loop {
        let mut changed = false;
        for i in 0..count {
            let next: BTreeSet<usize> = reach[i]
                .iter()
                .flat_map(|&j| reach[j].iter().copied())
                .collect();
            for j in next {
                changed |= reach[i].insert(j);
            }
        }
        if !changed {
            break;
        }
    }
    let mut pinned = vec![false; count];
    for (i, routine) in module.routines.iter().enumerate() {
        if matches!(routine.abi, Abi::Interrupt) {
            pinned[i] = true;
            for &j in &reach[i] {
                pinned[j] = true;
            }
        }
    }
    let interferes = |a: usize, b: usize| a == b || reach[a].contains(&b) || reach[b].contains(&a);

    // Slots owned by exactly one routine.
    let mut owners: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, layout) in assignment.routines.iter().enumerate() {
        for slot in &layout.direct_slots {
            owners.entry(slot.symbol.as_str()).or_default().push(i);
        }
    }
    let layout_routine = |i: usize| index_of(&assignment.routines[i].name);
    let mut blocks: Vec<(usize, Vec<(String, u16)>)> = Vec::new();
    for (i, layout) in assignment.routines.iter().enumerate() {
        let Some(routine) = layout_routine(i) else {
            continue;
        };
        if pinned[routine] {
            continue;
        }
        let mut slots: Vec<(String, u16)> = Vec::new();
        for slot in &layout.direct_slots {
            let private = owners[slot.symbol.as_str()].iter().all(|&owner| owner == i);
            if private && !slots.iter().any(|(symbol, _)| *symbol == slot.symbol) {
                slots.push((slot.symbol.clone(), slot.size));
            }
        }
        if !slots.is_empty() {
            blocks.push((routine, slots));
        }
    }
    let block_size = |slots: &[(String, u16)]| slots.iter().map(|(_, size)| size).sum::<u16>();
    blocks.sort_by_key(|(routine, slots)| (std::cmp::Reverse(block_size(slots)), *routine));
    let mut placed: Vec<(usize, u16, u16)> = Vec::new();
    let mut overlay = BTreeMap::new();
    let mut overlay_bytes = 0u16;
    let mut overlaid_bytes = 0u16;
    for (routine, slots) in &blocks {
        let size = block_size(slots);
        let mut offset = 0u16;
        while let Some(&(_, start, len)) = placed.iter().find(|&&(other, start, len)| {
            interferes(*routine, other) && offset < start + len && start < offset + size
        }) {
            offset = start + len;
        }
        placed.push((*routine, offset, size));
        let mut at = offset;
        for (symbol, slot_size) in slots {
            overlay.insert(symbol.clone(), at);
            at += slot_size;
        }
        overlay_bytes = overlay_bytes.max(offset + size);
        overlaid_bytes += size;
    }
    assignment.direct_page_bytes = assignment.direct_page_bytes - overlaid_bytes + overlay_bytes;
    assignment.overlay = overlay;
    assignment.overlay_bytes = overlay_bytes;
}

pub fn validate_storage_assignment(
    module: &Module,
    assignment: &StorageAssignment,
) -> Result<(), StorageAssignmentError> {
    let mut missing = Vec::new();

    for routine in &module.routines {
        let Some(layout) = assignment.routine(&routine.name) else {
            missing.push(MissingStorageBinding {
                routine: routine.name.clone(),
                key: BindingKey::Local("<routine layout>".to_owned()),
            });
            continue;
        };

        for key in referenced_bindings(routine) {
            if layout.location(key.clone()).is_none() {
                missing.push(MissingStorageBinding {
                    routine: routine.name.clone(),
                    key,
                });
            }
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(StorageAssignmentError { missing })
    }
}

pub fn apply_storage(module: &Module, assignment: &StorageAssignment) -> Module {
    debug_assert!(
        validate_storage_assignment(module, assignment).is_ok(),
        "storage assignment must cover all referenced params, locals, and temps"
    );

    let direct_aliases = direct_symbol_aliases(assignment);
    let routines = module
        .routines
        .iter()
        .map(|routine| {
            let Some(layout) = assignment.routine(&routine.name) else {
                return routine.clone();
            };

            let mut routine = routine.clone();
            let zeroing = layout
                .zeroed_registers
                .iter()
                .map(|(symbol, bytes)| IrInst::LoadConst {
                    dst: Value::Direct(symbol.clone()),
                    value: match bytes {
                        1 => Const::U8(0),
                        2 => Const::U16(0),
                        3 => Const::U24(0),
                        _ => Const::U32(0),
                    },
                });
            routine.body = zeroing
                .chain(
                    routine
                        .body
                        .iter()
                        .map(|inst| rewrite_inst(inst, layout, &direct_aliases)),
                )
                .collect();
            routine.stack_frame_bytes = layout.stack_frame_bytes;
            routine
        })
        .collect();

    Module {
        name: module.name.clone(),
        globals: module.globals.clone(),
        static_data: module.static_data.clone(),
        vectors: module.vectors.clone(),
        metadata: module.metadata.clone(),
        imports: module.imports.clone(),
        libraries: module.libraries.clone(),
        external_routines: module.external_routines.clone(),
        external_direct_symbols: module.external_direct_symbols.clone(),
        external_data: module.external_data.clone(),
        entry: module.entry.clone(),
        routines,
    }
}

fn direct_symbol_aliases(assignment: &StorageAssignment) -> BTreeMap<String, String> {
    assignment
        .routines
        .iter()
        .flat_map(|routine| {
            routine.direct_slots.iter().filter_map(|slot| {
                let BindingKey::Param(param) = &slot.source else {
                    return None;
                };
                let original = param_symbol(&routine.name, param);
                if original == slot.symbol {
                    None
                } else {
                    Some((original, slot.symbol.clone()))
                }
            })
        })
        .collect()
}

fn assign_routine(
    routine: &Routine,
    allocator: &mut DirectAllocator,
    args: &mut DirectSlotPool,
    scratch: &mut DirectSlotPool,
) -> RoutineStorage {
    let mut out = RoutineStorage {
        name: routine.name.clone(),
        direct_slots: Vec::new(),
        static_locals: Vec::new(),
        stack_slots: Vec::new(),
        stack_frame_bytes: 0,
        zeroed_registers: Vec::new(),
        bindings: BTreeMap::new(),
    };

    if routine.abi.stack_arguments() {
        let mut registers = allocator
            .temp_registers
            .map(|(symbol, bytes)| TempRegisters::new(symbol, bytes));
        assign_locals(routine, allocator, registers.as_mut(), &mut out);
        assign_scratch_bindings(routine, allocator, scratch, registers.as_mut(), &mut out);
        assign_stack_params(routine, &mut out);
    } else {
        // Interrupt handlers keep private slots rather than save the temp
        // registers.
        let mut registers = allocator
            .temp_registers
            .filter(|_| !matches!(routine.abi, Abi::Interrupt))
            .map(|(symbol, bytes)| TempRegisters::new(symbol, bytes));
        assign_params(routine, allocator, args, &mut out);
        assign_locals(routine, allocator, registers.as_mut(), &mut out);
        assign_scratch_bindings(routine, allocator, scratch, registers.as_mut(), &mut out);
    }

    out
}

fn assign_params(
    routine: &Routine,
    allocator: &mut DirectAllocator,
    args: &mut DirectSlotPool,
    out: &mut RoutineStorage,
) {
    let lifetimes = param_lifetimes(routine);
    let mut used_arg_symbols = Vec::new();
    let can_share_params = !routine.export;
    for param in &routine.params {
        let lifetime = lifetimes
            .iter()
            .find(|lifetime| lifetime.name == param.name)
            .expect("every parameter should have a lifetime");
        if !can_share_params || lifetime.crosses_call {
            allocate_direct_param(routine, param, allocator, out);
        } else {
            allocate_volatile_param(param, args, allocator, &mut used_arg_symbols, out);
        }
    }
}

fn assign_stack_params(routine: &Routine, out: &mut RoutineStorage) {
    let mut stack_offset = out.stack_frame_bytes + 3;
    for param in &routine.params {
        let size = param.ty.size_bytes();
        out.bindings.insert(
            BindingKey::Param(param.name.clone()),
            Location::Stack {
                offset: stack_offset,
                size,
            },
        );
        out.stack_slots.push(StackSlot {
            name: param.name.clone(),
            offset: stack_offset,
            size,
        });
        stack_offset += size;
    }
}

/// Whether a local is a pointer the CPU can only follow from the direct
/// page (`(zp),z` or `[zp],z`): kept anywhere else, every use first copies
/// it there.
fn wants_direct_page(routine: &Routine, local: &Local) -> bool {
    matches!(local.storage, Storage::Default)
        && !matches!(routine.abi, Abi::Reentrant)
        && matches!(
            local.ty,
            Type::Pointer {
                kind: PointerKind::Far | PointerKind::Near,
                ..
            }
        )
}

fn assign_locals(
    routine: &Routine,
    allocator: &mut DirectAllocator,
    mut registers: Option<&mut TempRegisters>,
    out: &mut RoutineStorage,
) {
    let mut stack_offset = 1;

    // Pointers first, into direct-page slots while the budget lasts (the
    // temp registers stay with other values, which they serve as well);
    // past the budget they take their chances below like any other local.
    let mut placed = BTreeSet::new();
    for local in routine.locals.iter().filter(|local| wants_direct_page(routine, local)) {
        if allocator.can_alloc(local.ty.size_bytes()) {
            allocate_direct_local(routine, local, allocator, out);
            placed.insert(local.name.clone());
        }
    }

    for local in routine.locals.iter().filter(|local| !placed.contains(&local.name)) {
        // A scalar local that never lives across a call can use the temp
        // registers instead of the frame or static storage. (Locals start
        // undefined, except in reentrant routines, where they start at zero.)
        if let (Some(registers), Storage::Default) = (registers.as_deref_mut(), local.storage) {
            let size = local.ty.size_bytes();
            let lifetime = (size <= 4)
                .then(|| local_lifetime(routine, &local.name))
                .flatten();
            if let Some((first_use, last_use, false)) = lifetime {
                if let Some(symbol) = registers.place(first_use, last_use, size) {
                    out.bindings.insert(
                        BindingKey::Local(local.name.clone()),
                        Location::Direct {
                            symbol: symbol.clone(),
                            offset: 0,
                            size,
                        },
                    );
                    // Reentrant locals start at zero, like the frame they replace.
                    if matches!(routine.abi, Abi::Reentrant) {
                        out.zeroed_registers.push((symbol, size));
                    }
                    continue;
                }
            }
        }
        match local.storage {
            Storage::Direct => allocate_direct_local(routine, local, allocator, out),
            Storage::Static => {
                let symbol = static_symbol(&routine.name, &local.name);
                let size = local.ty.size_bytes();
                out.bindings.insert(
                    BindingKey::Local(local.name.clone()),
                    Location::Static {
                        symbol: symbol.clone(),
                        size,
                    },
                );
                out.static_locals.push(StaticLocal {
                    name: local.name.clone(),
                    symbol,
                    size,
                });
            }
            Storage::Default if !matches!(routine.abi, Abi::Reentrant) => {
                let symbol = static_symbol(&routine.name, &local.name);
                let size = local.ty.size_bytes();
                out.bindings.insert(
                    BindingKey::Local(local.name.clone()),
                    Location::Static {
                        symbol: symbol.clone(),
                        size,
                    },
                );
                out.static_locals.push(StaticLocal {
                    name: local.name.clone(),
                    symbol,
                    size,
                });
            }
            Storage::Stack | Storage::Default => {
                let size = local.ty.size_bytes();
                out.bindings.insert(
                    BindingKey::Local(local.name.clone()),
                    Location::Stack {
                        offset: stack_offset,
                        size,
                    },
                );
                out.stack_slots.push(StackSlot {
                    name: local.name.clone(),
                    offset: stack_offset,
                    size,
                });
                stack_offset += size;
                out.stack_frame_bytes += size;
            }
            Storage::Scratch | Storage::Far | Storage::Bank(_) => {}
        }
    }
}

fn assign_scratch_bindings(
    routine: &Routine,
    allocator: &mut DirectAllocator,
    scratch: &mut DirectSlotPool,
    mut registers: Option<&mut TempRegisters>,
    out: &mut RoutineStorage,
) {
    let mut lifetimes = scratch_lifetimes(routine);
    if !allocator.wide_scratch_direct {
        let required = direct_required_bindings(routine);
        for binding in &mut lifetimes {
            binding.direct_required = required.contains(&binding.key);
        }
    }
    lifetimes.sort_by_key(|binding| {
        (
            binding.first_use,
            binding.last_use,
            binding.symbol_name.clone(),
        )
    });

    // Temporaries live within one statement, so their linear lifetime is
    // exact (unlike variables, which loops can keep alive around a back
    // edge): those that do not live across a call can use temp registers.
    let mut lifetimes: Vec<ScratchLifetime> = lifetimes
        .into_iter()
        .filter(|binding| {
            if !matches!(binding.key, BindingKey::Temp(_)) || binding.crosses_call {
                return true;
            }
            let Some(symbol) = registers.as_deref_mut().and_then(|registers| {
                registers.place(binding.first_use, binding.last_use, binding.size)
            }) else {
                return true;
            };
            out.bindings.insert(
                binding.key.clone(),
                Location::Direct {
                    symbol,
                    offset: 0,
                    size: binding.size,
                },
            );
            false
        })
        .collect();

    let framed = match routine.abi {
        Abi::Reentrant => true,
        Abi::Stack => allocator.cheap_frames,
        _ => false,
    };
    if framed {
        for binding in lifetimes.drain(..) {
            let offset = out.stack_frame_bytes + 1;
            out.bindings.insert(
                binding.key,
                Location::Stack {
                    offset,
                    size: binding.size,
                },
            );
            out.stack_slots.push(StackSlot {
                name: binding.symbol_name,
                offset,
                size: binding.size,
            });
            out.stack_frame_bytes += binding.size;
        }
        return;
    }

    let mut volatile_lifetimes = Vec::new();
    let mut private_lifetimes = Vec::new();
    for binding in lifetimes {
        if binding.crosses_call {
            private_lifetimes.push(binding);
        } else {
            volatile_lifetimes.push(binding);
        }
    }

    assign_volatile_scratch_bindings(volatile_lifetimes, allocator, scratch, out);
    assign_private_scratch_bindings(private_lifetimes, allocator, out);
}

/// Byte ranges of the temp-register block, handed out by lifetime.
struct TempRegisters {
    symbol: &'static str,
    bytes: u16,
    /// (offset, size, first use, last use) of the values placed so far.
    placed: Vec<(u16, u16, usize, usize)>,
}

impl TempRegisters {
    fn new(symbol: &'static str, bytes: u16) -> Self {
        Self {
            symbol,
            bytes,
            placed: Vec::new(),
        }
    }

    /// Places a value live from `first_use` to `last_use` (instruction
    /// indexes); returns its symbol, or `None` when the block is full then.
    fn place(&mut self, first_use: usize, last_use: usize, size: u16) -> Option<String> {
        let mut offset = 0;
        while offset + size <= self.bytes {
            let clash = self.placed.iter().find(|(start, bytes, first, last)| {
                offset < start + bytes
                    && *start < offset + size
                    && *first <= last_use
                    && first_use <= *last
            });
            match clash {
                Some((start, bytes, _, _)) => offset = start + bytes,
                None => {
                    self.placed.push((offset, size, first_use, last_use));
                    return Some(if offset == 0 {
                        self.symbol.to_owned()
                    } else {
                        format!("{}+{offset}", self.symbol)
                    });
                }
            }
        }
        None
    }
}

/// The span of instructions over which local `name` may be live, and
/// whether a call happens inside it. A loop that touches the local anywhere
/// keeps it live for the whole loop (a value can flow around the back
/// edge). Returns `None` for an unused local.
fn local_lifetime(routine: &Routine, name: &str) -> Option<(usize, usize, bool)> {
    let uses: Vec<usize> = routine
        .body
        .iter()
        .enumerate()
        .filter(|(_, inst)| local_uses(inst).iter().any(|used| used == name))
        .map(|(index, _)| index)
        .collect();
    let (mut first, mut last) = (*uses.first()?, *uses.last()?);
    let labels: HashMap<&Label, usize> = routine
        .body
        .iter()
        .enumerate()
        .filter_map(|(index, inst)| match inst {
            IrInst::Label(label) => Some((label, index)),
            _ => None,
        })
        .collect();
    let loops: Vec<(usize, usize)> = routine
        .body
        .iter()
        .enumerate()
        .filter_map(|(index, inst)| {
            let target = branch_target(inst)?;
            let head = *labels.get(target)?;
            (head <= index).then_some((head, index))
        })
        .collect();
    loop {
        let mut changed = false;
        for &(head, tail) in &loops {
            if head <= last && first <= tail && (head < first || last < tail) {
                first = first.min(head);
                last = last.max(tail);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let crosses_call = routine.body[first..=last].iter().any(is_call_barrier);
    Some((first, last, crosses_call))
}

fn branch_target(inst: &IrInst) -> Option<&Label> {
    match inst {
        IrInst::Branch { target }
        | IrInst::BranchIfZero { target, .. }
        | IrInst::BranchIfEqual { target }
        | IrInst::BranchIfNotEqual { target }
        | IrInst::BranchIfCarrySet { target }
        | IrInst::BranchIfCarryClear { target } => Some(target),
        _ => None,
    }
}

fn assign_volatile_scratch_bindings(
    lifetimes: Vec<ScratchLifetime>,
    allocator: &mut DirectAllocator,
    scratch: &mut DirectSlotPool,
    out: &mut RoutineStorage,
) {
    let direct_keys = direct_scratch_keys(&lifetimes, &scratch.slots, allocator.remaining_budget());
    let mut active: Vec<ActiveDirectSlot> = Vec::new();
    let mut free = scratch.slots.clone();

    for temp in lifetimes {
        if !direct_keys.contains(&temp.key) {
            bind_stack_scratch(out, &temp);
            continue;
        }

        let mut i = 0;
        while i < active.len() {
            if active[i].last_use < temp.first_use {
                let old = active.remove(i);
                free.push(AllocatedSlot {
                    symbol: old.symbol,
                    offset: old.offset,
                    size: old.size,
                });
            } else {
                i += 1;
            }
        }

        let slot_index = free.iter().position(|slot| slot.size >= temp.size);
        let slot = match slot_index {
            Some(index) => free.remove(index),
            None if allocator.can_alloc(temp.size) || temp.direct_required => {
                scratch.alloc(allocator, temp.size)
            }
            None => {
                bind_stack_scratch(out, &temp);
                continue;
            }
        };

        bind_direct_scratch(out, &temp, &slot);
        active.push(ActiveDirectSlot {
            symbol: slot.symbol,
            offset: slot.offset,
            size: slot.size,
            last_use: temp.last_use,
        });
    }
}

fn assign_private_scratch_bindings(
    lifetimes: Vec<ScratchLifetime>,
    allocator: &mut DirectAllocator,
    out: &mut RoutineStorage,
) {
    let direct_keys = direct_scratch_keys(&lifetimes, &[], allocator.remaining_budget());
    let mut active: Vec<ActiveDirectSlot> = Vec::new();
    let mut free: Vec<AllocatedSlot> = Vec::new();

    for temp in lifetimes {
        if !direct_keys.contains(&temp.key) {
            bind_stack_scratch(out, &temp);
            continue;
        }

        let mut i = 0;
        while i < active.len() {
            if active[i].last_use < temp.first_use {
                let old = active.remove(i);
                free.push(AllocatedSlot {
                    symbol: old.symbol,
                    offset: old.offset,
                    size: old.size,
                });
            } else {
                i += 1;
            }
        }

        let slot_index = free.iter().position(|slot| slot.size >= temp.size);
        let slot = match slot_index {
            Some(index) => free.remove(index),
            None if allocator.can_alloc(temp.size) || temp.direct_required => {
                allocator.alloc(temp.symbol_name.clone(), temp.size)
            }
            None => {
                bind_stack_scratch(out, &temp);
                continue;
            }
        };

        bind_direct_scratch(out, &temp, &slot);
        active.push(ActiveDirectSlot {
            symbol: slot.symbol,
            offset: slot.offset,
            size: slot.size,
            last_use: temp.last_use,
        });
    }
}

fn direct_scratch_keys(
    lifetimes: &[ScratchLifetime],
    reusable_slots: &[AllocatedSlot],
    new_budget: Option<u16>,
) -> BTreeSet<BindingKey> {
    let Some(mut new_budget) = new_budget else {
        return lifetimes
            .iter()
            .map(|binding| binding.key.clone())
            .collect();
    };

    let mut slots = reusable_slots
        .iter()
        .map(|slot| PlannedDirectSlot {
            size: slot.size,
            intervals: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut candidates = lifetimes.iter().collect::<Vec<_>>();
    candidates.sort_by_key(|binding| {
        (
            !binding.direct_required,
            std::cmp::Reverse(binding.use_count),
            binding.first_use,
            binding.last_use,
            binding.symbol_name.clone(),
        )
    });

    let mut direct = BTreeSet::new();
    for binding in candidates {
        if let Some(slot) = slots
            .iter_mut()
            .find(|slot| slot.can_place(binding.first_use, binding.last_use, binding.size))
        {
            slot.place(binding.first_use, binding.last_use);
            direct.insert(binding.key.clone());
        } else if binding.direct_required || binding.size <= new_budget {
            new_budget = new_budget.saturating_sub(binding.size);
            slots.push(PlannedDirectSlot {
                size: binding.size,
                intervals: vec![(binding.first_use, binding.last_use)],
            });
            direct.insert(binding.key.clone());
        }
    }

    direct
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedDirectSlot {
    size: u16,
    intervals: Vec<(usize, usize)>,
}

impl PlannedDirectSlot {
    fn can_place(&self, first_use: usize, last_use: usize, size: u16) -> bool {
        self.size >= size
            && self
                .intervals
                .iter()
                .all(|(first, last)| *last < first_use || last_use < *first)
    }

    fn place(&mut self, first_use: usize, last_use: usize) {
        self.intervals.push((first_use, last_use));
    }
}

fn bind_direct_scratch(out: &mut RoutineStorage, binding: &ScratchLifetime, slot: &AllocatedSlot) {
    out.bindings.insert(
        binding.key.clone(),
        Location::Direct {
            symbol: slot.symbol.clone(),
            offset: slot.offset,
            size: slot.size,
        },
    );
    out.direct_slots.push(DirectSlot {
        symbol: slot.symbol.clone(),
        offset: slot.offset,
        size: slot.size,
        source: binding.key.clone(),
    });
}

fn bind_stack_scratch(out: &mut RoutineStorage, binding: &ScratchLifetime) {
    let offset = out.stack_frame_bytes + 1;
    out.bindings.insert(
        binding.key.clone(),
        Location::Stack {
            offset,
            size: binding.size,
        },
    );
    out.stack_slots.push(StackSlot {
        name: binding.symbol_name.clone(),
        offset,
        size: binding.size,
    });
    out.stack_frame_bytes += binding.size;
}

fn allocate_direct_param(
    routine: &Routine,
    param: &Param,
    allocator: &mut DirectAllocator,
    out: &mut RoutineStorage,
) {
    let size = param.ty.size_bytes();
    let slot = allocator.alloc(param_symbol(&routine.name, &param.name), size);
    out.bindings.insert(
        BindingKey::Param(param.name.clone()),
        Location::Direct {
            symbol: slot.symbol.clone(),
            offset: slot.offset,
            size,
        },
    );
    out.direct_slots.push(DirectSlot {
        symbol: slot.symbol,
        offset: slot.offset,
        size,
        source: BindingKey::Param(param.name.clone()),
    });
}

fn allocate_volatile_param(
    param: &Param,
    args: &mut DirectSlotPool,
    allocator: &mut DirectAllocator,
    used_arg_symbols: &mut Vec<String>,
    out: &mut RoutineStorage,
) {
    let size = param.ty.size_bytes();
    let slot = args.alloc_excluding(allocator, size, used_arg_symbols);
    used_arg_symbols.push(slot.symbol.clone());
    out.bindings.insert(
        BindingKey::Param(param.name.clone()),
        Location::Direct {
            symbol: slot.symbol.clone(),
            offset: slot.offset,
            size: slot.size,
        },
    );
    out.direct_slots.push(DirectSlot {
        symbol: slot.symbol,
        offset: slot.offset,
        size: slot.size,
        source: BindingKey::Param(param.name.clone()),
    });
}

fn allocate_direct_local(
    routine: &Routine,
    local: &Local,
    allocator: &mut DirectAllocator,
    out: &mut RoutineStorage,
) {
    let size = local.ty.size_bytes();
    let slot = allocator.alloc(local_symbol(&routine.name, &local.name), size);
    out.bindings.insert(
        BindingKey::Local(local.name.clone()),
        Location::Direct {
            symbol: slot.symbol.clone(),
            offset: slot.offset,
            size,
        },
    );
    out.direct_slots.push(DirectSlot {
        symbol: slot.symbol,
        offset: slot.offset,
        size,
        source: BindingKey::Local(local.name.clone()),
    });
}

fn rewrite_inst(
    inst: &IrInst,
    layout: &RoutineStorage,
    direct_aliases: &BTreeMap<String, String>,
) -> IrInst {
    match inst {
        IrInst::InlineAsm(_) => inst.clone(),
        IrInst::Move { width, dst, src } => IrInst::Move {
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            src: rewrite_value(src, layout, direct_aliases),
        },
        IrInst::LoadConst { dst, value } => IrInst::LoadConst {
            dst: rewrite_value(dst, layout, direct_aliases),
            value: *value,
        },
        IrInst::LoadAddress { dst, symbol, width } => IrInst::LoadAddress {
            dst: rewrite_value(dst, layout, direct_aliases),
            symbol: symbol.clone(),
            width: *width,
        },
        IrInst::Load { width, dst, addr } => IrInst::Load {
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            addr: rewrite_address(addr, layout, direct_aliases),
        },
        IrInst::Store { width, addr, src } => IrInst::Store {
            width: *width,
            addr: rewrite_address(addr, layout, direct_aliases),
            src: rewrite_value(src, layout, direct_aliases),
        },
        IrInst::Add {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::Add {
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::Sub {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::Sub {
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::BitAnd {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::BitAnd {
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::BitOr {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::BitOr {
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::ShiftLeft {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::ShiftLeft {
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::ShiftRight {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::ShiftRight {
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::Dec { width, value } => IrInst::Dec {
            width: *width,
            value: rewrite_value(value, layout, direct_aliases),
        },
        IrInst::Inc { width, value } => IrInst::Inc {
            width: *width,
            value: rewrite_value(value, layout, direct_aliases),
        },
        IrInst::Compare {
            width,
            signed,
            lhs,
            rhs,
        } => IrInst::Compare {
            width: *width,
            signed: *signed,
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::Binary {
            op,
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::Binary {
            op: *op,
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::SignExtend {
            width,
            src_width,
            dst,
            src,
        } => IrInst::SignExtend {
            width: *width,
            src_width: *src_width,
            dst: rewrite_value(dst, layout, direct_aliases),
            src: rewrite_value(src, layout, direct_aliases),
        },
        IrInst::CallIndirect {
            target,
            abi,
            result,
        } => IrInst::CallIndirect {
            target: rewrite_value(target, layout, direct_aliases),
            abi: *abi,
            result: result
                .as_ref()
                .map(|(width, dst)| (*width, rewrite_value(dst, layout, direct_aliases))),
        },
        IrInst::BranchIfZero { value, target } => IrInst::BranchIfZero {
            value: rewrite_value(value, layout, direct_aliases),
            target: target.clone(),
        },
        IrInst::Push { width, value } => IrInst::Push {
            width: *width,
            value: rewrite_value(value, layout, direct_aliases),
        },
        IrInst::CallFunction {
            name,
            abi,
            width,
            dst,
        } => IrInst::CallFunction {
            name: name.clone(),
            abi: *abi,
            width: *width,
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        IrInst::CopSignature { dst } => IrInst::CopSignature {
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        IrInst::CopRegister { register, dst } => IrInst::CopRegister {
            register: *register,
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        IrInst::Trap {
            family,
            call,
            x,
            y,
            dst,
        } => IrInst::Trap {
            family: *family,
            call: rewrite_value(call, layout, direct_aliases),
            x: rewrite_value(x, layout, direct_aliases),
            y: rewrite_value(y, layout, direct_aliases),
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        IrInst::ReturnValue(value) => {
            IrInst::ReturnValue(rewrite_value(value, layout, direct_aliases))
        }
        IrInst::LoadRegisters { args } => IrInst::LoadRegisters {
            args: args
                .iter()
                .map(|(register, value)| (*register, rewrite_value(value, layout, direct_aliases)))
                .collect(),
        },
        IrInst::MoveCarry { dst } => IrInst::MoveCarry {
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        other => other.clone(),
    }
}

fn rewrite_address(
    addr: &Address,
    layout: &RoutineStorage,
    direct_aliases: &BTreeMap<String, String>,
) -> Address {
    match addr {
        Address::ValueOffset { base, offset } => Address::ValueOffset {
            base: rewrite_value(base, layout, direct_aliases),
            offset: *offset,
        },
        Address::NearPtr(value) => Address::NearPtr(rewrite_value(value, layout, direct_aliases)),
        Address::FarPtr(value) => Address::FarPtr(rewrite_value(value, layout, direct_aliases)),
        Address::PointerOffset {
            pointer,
            far,
            offset,
        } => Address::PointerOffset {
            pointer: rewrite_value(pointer, layout, direct_aliases),
            far: *far,
            offset: *offset,
        },
        Address::Indexed {
            base,
            index,
            byte_offset,
        } => Address::Indexed {
            base: base.clone(),
            index: rewrite_value(index, layout, direct_aliases),
            byte_offset: *byte_offset,
        },
        other => other.clone(),
    }
}

fn rewrite_value(
    value: &Value,
    layout: &RoutineStorage,
    direct_aliases: &BTreeMap<String, String>,
) -> Value {
    if let Value::Direct(name) = value {
        if let Some(alias) = direct_aliases.get(name) {
            return Value::Direct(alias.clone());
        }
    }

    let key = match value {
        Value::Param(name) => Some(BindingKey::Param(name.clone())),
        Value::Local(name) => Some(BindingKey::Local(name.clone())),
        Value::Temp(name) => Some(BindingKey::Temp(name.clone())),
        _ => None,
    };

    let Some(key) = key else {
        return value.clone();
    };

    match layout.location(key) {
        Some(Location::Direct { symbol, .. }) => Value::Direct(symbol.clone()),
        Some(Location::Static { symbol, .. }) => Value::Local(symbol.clone()),
        Some(Location::Stack { offset, size }) => Value::Stack {
            offset: *offset,
            size: *size,
        },
        None => value.clone(),
    }
}

fn param_lifetimes(routine: &Routine) -> Vec<ParamLifetime> {
    let mut params = routine
        .params
        .iter()
        .map(|param| (param.name.clone(), (None::<usize>, None::<usize>)))
        .collect::<BTreeMap<_, _>>();
    let mut call_indices = Vec::new();

    for (index, inst) in routine.body.iter().enumerate() {
        if is_call_barrier(inst) {
            call_indices.push(index);
        }
        for name in param_uses(inst) {
            params
                .entry(name)
                .and_modify(|entry| {
                    entry.0 = Some(entry.0.unwrap_or(0).min(index));
                    entry.1 = Some(index);
                })
                .or_insert((Some(0), Some(index)));
        }
    }

    params
        .into_iter()
        .map(|(name, (first_use, last_use))| {
            // A parameter is live from entry (the caller stored it), not
            // from its first use.
            let crosses_call = match (first_use, last_use) {
                (Some(_), Some(last_use)) => call_indices.iter().any(|call| *call < last_use),
                _ => false,
            };
            ParamLifetime { name, crosses_call }
        })
        .collect()
}

fn temp_lifetimes(routine: &Routine) -> Vec<TempLifetime> {
    let mut temps = BTreeMap::<String, (usize, usize, u16, usize)>::new();
    let mut call_indices = Vec::new();
    let direct_required = direct_required_bindings(routine);

    for (index, inst) in routine.body.iter().enumerate() {
        if is_call_barrier(inst) {
            call_indices.push(index);
        }
        for (name, size) in temp_uses(inst) {
            temps
                .entry(name)
                .and_modify(|entry| {
                    entry.1 = index;
                    entry.2 = entry.2.max(size);
                    entry.3 += 1;
                })
                .or_insert((index, index, size, 1));
        }
    }

    temps
        .into_iter()
        .map(|(name, (first_use, last_use, size, use_count))| {
            let requires_direct =
                size > 2 || direct_required.contains(&BindingKey::Temp(name.clone()));
            TempLifetime {
                name,
                first_use,
                last_use,
                size,
                use_count,
                direct_required: requires_direct,
                crosses_call: call_indices
                    .iter()
                    .any(|call| first_use < *call && *call < last_use),
            }
        })
        .collect()
}

fn referenced_bindings(routine: &Routine) -> BTreeSet<BindingKey> {
    let mut out = BTreeSet::new();

    for inst in &routine.body {
        out.extend(param_uses(inst).into_iter().map(BindingKey::Param));
        out.extend(local_uses(inst).into_iter().map(BindingKey::Local));
        out.extend(
            temp_uses(inst)
                .into_iter()
                .map(|(name, _size)| BindingKey::Temp(name)),
        );
    }

    out
}

fn scratch_lifetimes(routine: &Routine) -> Vec<ScratchLifetime> {
    let mut lifetimes = temp_lifetimes(routine)
        .into_iter()
        .map(|temp| ScratchLifetime {
            key: BindingKey::Temp(temp.name.clone()),
            symbol_name: temp_symbol(&routine.name, &temp.name),
            first_use: temp.first_use,
            last_use: temp.last_use,
            size: temp.size,
            use_count: temp.use_count,
            direct_required: temp.direct_required,
            crosses_call: temp.crosses_call,
        })
        .collect::<Vec<_>>();

    lifetimes.extend(scratch_local_lifetimes(routine));
    lifetimes
}

fn scratch_local_lifetimes(routine: &Routine) -> Vec<ScratchLifetime> {
    let scratch_locals = routine
        .locals
        .iter()
        .filter(|local| matches!(local.storage, Storage::Scratch))
        .map(|local| (local.name.clone(), local.ty.size_bytes()))
        .collect::<BTreeMap<_, _>>();
    if scratch_locals.is_empty() {
        return Vec::new();
    }

    let mut locals = scratch_locals
        .iter()
        .map(|(name, size)| (name.clone(), (None::<usize>, None::<usize>, *size, 0usize)))
        .collect::<BTreeMap<_, _>>();
    let mut call_indices = Vec::new();
    let direct_required = direct_required_bindings(routine);

    for (index, inst) in routine.body.iter().enumerate() {
        if is_call_barrier(inst) {
            call_indices.push(index);
        }
        for name in local_uses(inst) {
            if let Some(entry) = locals.get_mut(&name) {
                entry.0 = Some(entry.0.unwrap_or(index).min(index));
                entry.1 = Some(index);
                entry.3 += 1;
            }
        }
    }

    locals
        .into_iter()
        .filter_map(|(name, (first_use, last_use, size, use_count))| {
            let (Some(first_use), Some(last_use)) = (first_use, last_use) else {
                return None;
            };
            Some(ScratchLifetime {
                key: BindingKey::Local(name.clone()),
                symbol_name: local_symbol(&routine.name, &name),
                first_use,
                last_use,
                size,
                use_count,
                direct_required: size > 2
                    || direct_required.contains(&BindingKey::Local(name.clone())),
                crosses_call: call_indices
                    .iter()
                    .any(|call| first_use < *call && *call < last_use),
            })
        })
        .collect()
}

fn direct_required_bindings(routine: &Routine) -> BTreeSet<BindingKey> {
    let mut out = BTreeSet::new();

    for inst in &routine.body {
        match inst {
            IrInst::Load { addr, .. } | IrInst::Store { addr, .. } => {
                collect_direct_required_address(addr, &mut out);
            }
            _ => {}
        }
    }

    out
}

fn is_call_barrier(inst: &IrInst) -> bool {
    matches!(
        inst,
        IrInst::CallProcedure { .. }
            | IrInst::CallFunction { .. }
            | IrInst::CallIndirect { .. }
            | IrInst::Trap { .. }
    )
}

fn param_uses(inst: &IrInst) -> Vec<String> {
    let mut out = Vec::new();

    match inst {
        IrInst::LoadConst { dst, .. } => collect_param_value(dst, &mut out),
        IrInst::LoadAddress { dst, .. } => collect_param_value(dst, &mut out),
        IrInst::Move { dst, src, .. } => {
            collect_param_value(dst, &mut out);
            collect_param_value(src, &mut out);
        }
        IrInst::Load { dst, addr, .. } => {
            collect_param_value(dst, &mut out);
            collect_param_address(addr, &mut out);
        }
        IrInst::Store { addr, src, .. } => {
            collect_param_value(src, &mut out);
            collect_param_address(addr, &mut out);
        }
        IrInst::Add {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::Sub {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::BitAnd {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::BitOr {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::ShiftLeft {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::ShiftRight {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::Binary { dst, lhs, rhs, .. } => {
            collect_param_value(dst, &mut out);
            collect_param_value(lhs, &mut out);
            collect_param_value(rhs, &mut out);
        }
        IrInst::SignExtend { dst, src, .. } => {
            collect_param_value(dst, &mut out);
            collect_param_value(src, &mut out);
        }
        IrInst::CallIndirect { target, result, .. } => {
            collect_param_value(target, &mut out);
            if let Some((_, dst)) = result {
                collect_param_value(dst, &mut out);
            }
        }
        IrInst::LoadRegisters { args } => {
            for (_, value) in args {
                collect_param_value(value, &mut out);
            }
        }
        IrInst::MoveCarry { dst } => collect_param_value(dst, &mut out),
        IrInst::Dec { value, .. } | IrInst::Inc { value, .. } => {
            collect_param_value(value, &mut out);
        }
        IrInst::Compare { lhs, rhs, .. } => {
            collect_param_value(lhs, &mut out);
            collect_param_value(rhs, &mut out);
        }
        IrInst::BranchIfZero { value, .. } | IrInst::ReturnValue(value) => {
            collect_param_value(value, &mut out);
        }
        IrInst::Push { value, .. } => collect_param_value(value, &mut out),
        IrInst::CallFunction { dst, .. } => collect_param_value(dst, &mut out),
        IrInst::CopSignature { dst } => collect_param_value(dst, &mut out),
        IrInst::CopRegister { dst, .. } => collect_param_value(dst, &mut out),
        IrInst::Trap {
            call, x, y, dst, ..
        } => {
            collect_param_value(call, &mut out);
            collect_param_value(x, &mut out);
            collect_param_value(y, &mut out);
            collect_param_value(dst, &mut out);
        }
        IrInst::Label(_)
        | IrInst::InlineAsm(_)
        | IrInst::Branch { .. }
        | IrInst::BranchIfEqual { .. }
        | IrInst::BranchIfNotEqual { .. }
        | IrInst::BranchIfCarrySet { .. }
        | IrInst::BranchIfCarryClear { .. }
        | IrInst::CallProcedure { .. }
        | IrInst::CleanupStack { .. }
        | IrInst::Return => {}
    }

    out
}

fn temp_uses(inst: &IrInst) -> Vec<(String, u16)> {
    let mut out = Vec::new();

    match inst {
        IrInst::LoadConst { dst, value } => collect_temp_value(dst, const_size(*value), &mut out),
        IrInst::LoadAddress { dst, width, .. } => {
            collect_temp_value(dst, width.size_bytes(), &mut out)
        }
        IrInst::Move { width, dst, src } => {
            collect_temp_value(dst, width.size_bytes(), &mut out);
            collect_temp_value(src, width.size_bytes(), &mut out);
        }
        IrInst::Load { width, dst, addr } => {
            collect_temp_value(dst, width.size_bytes(), &mut out);
            collect_temp_address(addr, &mut out);
        }
        IrInst::Store { width, addr, src } => {
            collect_temp_value(src, width.size_bytes(), &mut out);
            collect_temp_address(addr, &mut out);
        }
        IrInst::Add {
            width,
            dst,
            lhs,
            rhs,
        }
        | IrInst::Sub {
            width,
            dst,
            lhs,
            rhs,
        }
        | IrInst::BitAnd {
            width,
            dst,
            lhs,
            rhs,
        }
        | IrInst::BitOr {
            width,
            dst,
            lhs,
            rhs,
        }
        | IrInst::ShiftLeft {
            width,
            dst,
            lhs,
            rhs,
        }
        | IrInst::ShiftRight {
            width,
            dst,
            lhs,
            rhs,
        }
        | IrInst::Binary {
            width,
            dst,
            lhs,
            rhs,
            ..
        } => {
            let size = width.size_bytes();
            collect_temp_value(dst, size, &mut out);
            collect_temp_value(lhs, size, &mut out);
            collect_temp_value(rhs, size, &mut out);
        }
        IrInst::SignExtend {
            width,
            src_width,
            dst,
            src,
        } => {
            collect_temp_value(dst, width.size_bytes(), &mut out);
            collect_temp_value(src, src_width.size_bytes(), &mut out);
        }
        IrInst::CallIndirect { target, result, .. } => {
            collect_temp_value(target, 2, &mut out);
            if let Some((width, dst)) = result {
                collect_temp_value(dst, width.size_bytes(), &mut out);
            }
        }
        IrInst::LoadRegisters { args } => {
            for (register, value) in args {
                collect_temp_value(value, register.bytes(), &mut out);
            }
        }
        IrInst::MoveCarry { dst } => collect_temp_value(dst, 1, &mut out),
        IrInst::Dec { width, value } | IrInst::Inc { width, value } => {
            collect_temp_value(value, width.size_bytes(), &mut out);
        }
        IrInst::Compare {
            width, lhs, rhs, ..
        } => {
            let size = width.size_bytes();
            collect_temp_value(lhs, size, &mut out);
            collect_temp_value(rhs, size, &mut out);
        }
        IrInst::BranchIfZero { value, .. } | IrInst::ReturnValue(value) => {
            collect_temp_value(value, 2, &mut out);
        }
        IrInst::Push { width, value } => collect_temp_value(value, width.size_bytes(), &mut out),
        IrInst::CallFunction { width, dst, .. } => {
            collect_temp_value(dst, width.size_bytes(), &mut out)
        }
        IrInst::CopSignature { dst } => collect_temp_value(dst, 1, &mut out),
        IrInst::CopRegister { dst, .. } => collect_temp_value(dst, 2, &mut out),
        IrInst::Trap {
            call, x, y, dst, ..
        } => {
            collect_temp_value(call, 2, &mut out);
            collect_temp_value(x, 2, &mut out);
            collect_temp_value(y, 2, &mut out);
            collect_temp_value(dst, 2, &mut out);
        }
        IrInst::Label(_)
        | IrInst::InlineAsm(_)
        | IrInst::Branch { .. }
        | IrInst::BranchIfEqual { .. }
        | IrInst::BranchIfNotEqual { .. }
        | IrInst::BranchIfCarrySet { .. }
        | IrInst::BranchIfCarryClear { .. }
        | IrInst::CallProcedure { .. }
        | IrInst::CleanupStack { .. }
        | IrInst::Return => {}
    }

    out
}

fn local_uses(inst: &IrInst) -> Vec<String> {
    let mut out = Vec::new();

    match inst {
        IrInst::LoadConst { dst, .. } => collect_local_value(dst, &mut out),
        IrInst::LoadAddress { dst, .. } => collect_local_value(dst, &mut out),
        IrInst::Move { dst, src, .. } => {
            collect_local_value(dst, &mut out);
            collect_local_value(src, &mut out);
        }
        IrInst::Load { dst, addr, .. } => {
            collect_local_value(dst, &mut out);
            collect_local_address(addr, &mut out);
        }
        IrInst::Store { addr, src, .. } => {
            collect_local_value(src, &mut out);
            collect_local_address(addr, &mut out);
        }
        IrInst::Add {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::Sub {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::BitAnd {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::BitOr {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::ShiftLeft {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::ShiftRight {
            width: _,
            dst,
            lhs,
            rhs,
        }
        | IrInst::Binary { dst, lhs, rhs, .. } => {
            collect_local_value(dst, &mut out);
            collect_local_value(lhs, &mut out);
            collect_local_value(rhs, &mut out);
        }
        IrInst::SignExtend { dst, src, .. } => {
            collect_local_value(dst, &mut out);
            collect_local_value(src, &mut out);
        }
        IrInst::CallIndirect { target, result, .. } => {
            collect_local_value(target, &mut out);
            if let Some((_, dst)) = result {
                collect_local_value(dst, &mut out);
            }
        }
        IrInst::LoadRegisters { args } => {
            for (_, value) in args {
                collect_local_value(value, &mut out);
            }
        }
        IrInst::MoveCarry { dst } => collect_local_value(dst, &mut out),
        IrInst::Dec { value, .. } | IrInst::Inc { value, .. } => {
            collect_local_value(value, &mut out);
        }
        IrInst::Compare { lhs, rhs, .. } => {
            collect_local_value(lhs, &mut out);
            collect_local_value(rhs, &mut out);
        }
        IrInst::BranchIfZero { value, .. } | IrInst::ReturnValue(value) => {
            collect_local_value(value, &mut out);
        }
        IrInst::Push { value, .. } => collect_local_value(value, &mut out),
        IrInst::CallFunction { dst, .. } => collect_local_value(dst, &mut out),
        IrInst::CopSignature { dst } => collect_local_value(dst, &mut out),
        IrInst::CopRegister { dst, .. } => collect_local_value(dst, &mut out),
        IrInst::Trap {
            call, x, y, dst, ..
        } => {
            collect_local_value(call, &mut out);
            collect_local_value(x, &mut out);
            collect_local_value(y, &mut out);
            collect_local_value(dst, &mut out);
        }
        IrInst::Label(_)
        | IrInst::InlineAsm(_)
        | IrInst::Branch { .. }
        | IrInst::BranchIfEqual { .. }
        | IrInst::BranchIfNotEqual { .. }
        | IrInst::BranchIfCarrySet { .. }
        | IrInst::BranchIfCarryClear { .. }
        | IrInst::CallProcedure { .. }
        | IrInst::CleanupStack { .. }
        | IrInst::Return => {}
    }

    out
}

fn collect_temp_address(addr: &Address, out: &mut Vec<(String, u16)>) {
    match addr {
        Address::ValueOffset { base, .. } => collect_temp_value(base, 2, out),
        Address::NearPtr(value) => collect_temp_value(value, 2, out),
        Address::FarPtr(value) => collect_temp_value(value, 4, out),
        Address::PointerOffset { pointer, far, .. } => {
            collect_temp_value(pointer, if *far { 4 } else { 2 }, out)
        }
        Address::Indexed { index, .. } => collect_temp_value(index, 2, out),
        _ => {}
    }
}

fn collect_direct_required_address(addr: &Address, out: &mut BTreeSet<BindingKey>) {
    match addr {
        Address::NearPtr(value)
        | Address::FarPtr(value)
        | Address::PointerOffset { pointer: value, .. } => {
            collect_direct_required_value(value, out);
        }
        _ => {}
    }
}

fn collect_direct_required_value(value: &Value, out: &mut BTreeSet<BindingKey>) {
    match value {
        Value::Local(name) => {
            out.insert(BindingKey::Local(name.clone()));
        }
        Value::Temp(name) => {
            out.insert(BindingKey::Temp(name.clone()));
        }
        _ => {}
    }
}

fn collect_local_address(addr: &Address, out: &mut Vec<String>) {
    match addr {
        Address::ValueOffset { base, .. } => collect_local_value(base, out),
        Address::NearPtr(value) => collect_local_value(value, out),
        Address::FarPtr(value) => collect_local_value(value, out),
        Address::PointerOffset { pointer, .. } => collect_local_value(pointer, out),
        Address::Indexed { index, .. } => collect_local_value(index, out),
        _ => {}
    }
}

fn collect_param_address(addr: &Address, out: &mut Vec<String>) {
    match addr {
        Address::ValueOffset { base, .. } => collect_param_value(base, out),
        Address::NearPtr(value) => collect_param_value(value, out),
        Address::FarPtr(value) => collect_param_value(value, out),
        Address::PointerOffset { pointer, .. } => collect_param_value(pointer, out),
        Address::Indexed { index, .. } => collect_param_value(index, out),
        _ => {}
    }
}

fn collect_local_value(value: &Value, out: &mut Vec<String>) {
    if let Value::Local(name) = value {
        out.push(name.clone());
    }
}

fn collect_temp_value(value: &Value, size: u16, out: &mut Vec<(String, u16)>) {
    if let Value::Temp(name) = value {
        out.push((name.clone(), size));
    }
}

fn collect_param_value(value: &Value, out: &mut Vec<String>) {
    if let Value::Param(name) = value {
        out.push(name.clone());
    }
}

fn const_size(value: Const) -> u16 {
    value.width().size_bytes()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParamLifetime {
    name: String,
    crosses_call: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TempLifetime {
    name: String,
    first_use: usize,
    last_use: usize,
    size: u16,
    use_count: usize,
    direct_required: bool,
    crosses_call: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScratchLifetime {
    key: BindingKey,
    symbol_name: String,
    first_use: usize,
    last_use: usize,
    size: u16,
    use_count: usize,
    direct_required: bool,
    crosses_call: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveDirectSlot {
    symbol: String,
    offset: u16,
    size: u16,
    last_use: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AllocatedSlot {
    symbol: String,
    offset: u16,
    size: u16,
}

struct DirectAllocator {
    next_offset: u16,
    budget: Option<u16>,
    wide_scratch_direct: bool,
    temp_registers: Option<(&'static str, u16)>,
    cheap_frames: bool,
}

impl DirectAllocator {
    fn new(budget: Option<u16>) -> Self {
        Self {
            next_offset: 0,
            budget,
            wide_scratch_direct: true,
            temp_registers: None,
            cheap_frames: true,
        }
    }

    fn can_alloc(&self, size: u16) -> bool {
        self.budget
            .map(|budget| self.next_offset.saturating_add(size) <= budget)
            .unwrap_or(true)
    }

    fn remaining_budget(&self) -> Option<u16> {
        self.budget
            .map(|budget| budget.saturating_sub(self.next_offset))
    }

    fn alloc(&mut self, symbol: String, size: u16) -> AllocatedSlot {
        let slot = AllocatedSlot {
            symbol,
            offset: self.next_offset,
            size,
        };
        self.next_offset += size;
        slot
    }
}

struct DirectSlotPool {
    prefix: String,
    slots: Vec<AllocatedSlot>,
}

impl DirectSlotPool {
    fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            slots: Vec::new(),
        }
    }

    fn alloc(&mut self, allocator: &mut DirectAllocator, size: u16) -> AllocatedSlot {
        let symbol = format!("{}_{}", self.prefix, self.slots.len());
        let slot = allocator.alloc(symbol, size);
        self.slots.push(slot.clone());
        slot
    }

    fn alloc_excluding(
        &mut self,
        allocator: &mut DirectAllocator,
        size: u16,
        used_symbols: &[String],
    ) -> AllocatedSlot {
        if let Some(slot) = self
            .slots
            .iter()
            .find(|slot| slot.size >= size && !used_symbols.contains(&slot.symbol))
        {
            return slot.clone();
        }

        self.alloc(allocator, size)
    }
}

fn local_symbol(routine: &str, name: &str) -> String {
    format!("__dp_{routine}_local_{name}")
}

fn temp_symbol(routine: &str, name: &str) -> String {
    format!("__dp_{routine}_temp_{name}")
}

fn static_symbol(routine: &str, name: &str) -> String {
    format!("__static_{routine}_{name}")
}

impl fmt::Display for StorageAssignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.report())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::ir::{
        Abi, Const, IrInst, Label, Local, Module, Routine, Storage, Type, Value, Width,
    };

    use super::{
        apply_storage, assign_storage, assign_storage_with_budget, assign_storage_with_options,
        validate_direct_page_budget, validate_storage_assignment, BindingKey, Location,
        RoutineStorage, StorageAssignment, StorageOptions,
    };

    #[test]
    fn allocates_direct_params_and_locals() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("copy")
                .with_param(crate::ir::Param::new("src", Type::near_ptr(Type::U8)).direct())
                .with_local(Local::new("count", Type::U16, Storage::Direct)),
        );

        let assignment = assign_storage(&module);
        let routine = assignment.routine("copy").unwrap();

        assert_eq!(assignment.direct_page_bytes, 4);
        assert!(matches!(
            routine.location(BindingKey::Param("src".to_owned())),
            Some(Location::Direct {
                offset: 0,
                size: 2,
                ..
            })
        ));
        assert!(matches!(
            routine.location(BindingKey::Local("count".to_owned())),
            Some(Location::Direct {
                offset: 2,
                size: 2,
                ..
            })
        ));
    }

    #[test]
    fn defaults_non_reentrant_locals_to_static_storage() {
        let module =
            Module::new("layout").with_routine(Routine::procedure("state").with_local(Local::new(
                "counter",
                Type::U16,
                Storage::Default,
            )));

        let assignment = assign_storage(&module);
        let routine = assignment.routine("state").unwrap();

        assert!(matches!(
            routine.location(BindingKey::Local("counter".to_owned())),
            Some(Location::Static { symbol, size: 2 }) if symbol == "__static_state_counter"
        ));
    }

    #[test]
    fn defaults_reentrant_locals_to_stack_storage() {
        let mut routine =
            Routine::procedure("parse").with_local(Local::new("pos", Type::U16, Storage::Default));
        routine.abi = crate::ir::Abi::Reentrant;
        let module = Module::new("layout").with_routine(routine);

        let assignment = assign_storage(&module);
        let routine = assignment.routine("parse").unwrap();

        assert!(matches!(
            routine.location(BindingKey::Local("pos".to_owned())),
            Some(Location::Stack { offset: 1, size: 2 })
        ));
        assert_eq!(routine.stack_frame_bytes, 2);
    }

    #[test]
    fn reuses_non_overlapping_scratch_temp_slots() {
        let module =
            Module::new("layout").with_routine(Routine::procedure("temps").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Temp("a".to_owned()),
                    value: Const::U16(1),
                },
                IrInst::Store {
                    width: Width::Word,
                    addr: crate::ir::Address::Direct("out".to_owned()),
                    src: Value::Temp("a".to_owned()),
                },
                IrInst::LoadConst {
                    dst: Value::Temp("b".to_owned()),
                    value: Const::U16(2),
                },
                IrInst::Store {
                    width: Width::Word,
                    addr: crate::ir::Address::Direct("out".to_owned()),
                    src: Value::Temp("b".to_owned()),
                },
            ]));

        let assignment = assign_storage(&module);
        let routine = assignment.routine("temps").unwrap();

        assert_eq!(assignment.direct_page_bytes, 2);
        assert_eq!(
            routine.location(BindingKey::Temp("a".to_owned())),
            routine.location(BindingKey::Temp("b".to_owned()))
        );
    }

    #[test]
    fn reuses_volatile_scratch_temp_slots_across_routines() {
        let module = Module::new("layout")
            .with_routine(Routine::procedure("first").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Temp("a".to_owned()),
                    value: Const::U16(1),
                },
                IrInst::Store {
                    width: Width::Word,
                    addr: crate::ir::Address::Direct("out".to_owned()),
                    src: Value::Temp("a".to_owned()),
                },
            ]))
            .with_routine(Routine::procedure("second").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Temp("b".to_owned()),
                    value: Const::U16(2),
                },
                IrInst::Store {
                    width: Width::Word,
                    addr: crate::ir::Address::Direct("out".to_owned()),
                    src: Value::Temp("b".to_owned()),
                },
            ]));

        let assignment = assign_storage(&module);
        let first = assignment.routine("first").unwrap();
        let second = assignment.routine("second").unwrap();

        assert_eq!(assignment.direct_page_bytes, 2);
        assert_eq!(
            first.location(BindingKey::Temp("a".to_owned())),
            second.location(BindingKey::Temp("b".to_owned()))
        );
    }

    #[test]
    fn keeps_call_live_scratch_temps_private() {
        let module = Module::new("layout")
            .with_routine(Routine::procedure("caller").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Temp("a".to_owned()),
                    value: Const::U16(1),
                },
                IrInst::CallProcedure {
                    name: "callee".to_owned(),
                    abi: Abi::Internal,
                },
                IrInst::Store {
                    width: Width::Word,
                    addr: crate::ir::Address::Direct("out".to_owned()),
                    src: Value::Temp("a".to_owned()),
                },
            ]))
            .with_routine(Routine::procedure("callee").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Temp("b".to_owned()),
                    value: Const::U16(2),
                },
                IrInst::Store {
                    width: Width::Word,
                    addr: crate::ir::Address::Direct("out".to_owned()),
                    src: Value::Temp("b".to_owned()),
                },
            ]));

        let assignment = assign_storage(&module);
        let caller = assignment.routine("caller").unwrap();
        let callee = assignment.routine("callee").unwrap();

        assert_eq!(assignment.direct_page_bytes, 4);
        assert_ne!(
            caller.location(BindingKey::Temp("a".to_owned())),
            callee.location(BindingKey::Temp("b".to_owned()))
        );
    }

    #[test]
    fn reuses_non_overlapping_scratch_local_slots() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("scratch_locals")
                .with_local(Local::new("first", Type::U16, Storage::Scratch))
                .with_local(Local::new("second", Type::U16, Storage::Scratch))
                .with_body(vec![
                    IrInst::LoadConst {
                        dst: Value::Local("first".to_owned()),
                        value: Const::U16(1),
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("first".to_owned()),
                    },
                    IrInst::LoadConst {
                        dst: Value::Local("second".to_owned()),
                        value: Const::U16(2),
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("second".to_owned()),
                    },
                ]),
        );

        let assignment = assign_storage(&module);
        let routine = assignment.routine("scratch_locals").unwrap();

        assert_eq!(assignment.direct_page_bytes, 2);
        assert_eq!(
            routine.location(BindingKey::Local("first".to_owned())),
            routine.location(BindingKey::Local("second".to_owned()))
        );
        assert!(matches!(
            routine.location(BindingKey::Local("first".to_owned())),
            Some(Location::Direct { symbol, .. }) if symbol == "__dp_scratch_0"
        ));
        validate_storage_assignment(&module, &assignment).unwrap();
    }

    #[test]
    fn spills_scratch_locals_to_stack_when_direct_page_budget_is_exhausted() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("scratch_pressure")
                .with_local(Local::new("reserved", Type::U16, Storage::Direct))
                .with_local(Local::new("scratch", Type::U16, Storage::Scratch))
                .with_body(vec![
                    IrInst::LoadConst {
                        dst: Value::Local("reserved".to_owned()),
                        value: Const::U16(1),
                    },
                    IrInst::LoadConst {
                        dst: Value::Local("scratch".to_owned()),
                        value: Const::U16(2),
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("scratch".to_owned()),
                    },
                ]),
        );

        let assignment = assign_storage_with_budget(&module, 2);
        let routine = assignment.routine("scratch_pressure").unwrap();

        assert_eq!(assignment.direct_page_bytes, 2);
        assert!(matches!(
            routine.location(BindingKey::Local("reserved".to_owned())),
            Some(Location::Direct {
                offset: 0,
                size: 2,
                ..
            })
        ));
        assert!(matches!(
            routine.location(BindingKey::Local("scratch".to_owned())),
            Some(Location::Stack { offset: 1, size: 2 })
        ));
        assert_eq!(routine.stack_frame_bytes, 2);
        validate_storage_assignment(&module, &assignment).unwrap();
    }

    #[test]
    fn keeps_indirect_pointer_scratch_local_direct_under_budget_pressure() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("scratch_pointer")
                .with_local(Local::new("reserved", Type::U16, Storage::Direct))
                .with_local(Local::new(
                    "ptr",
                    Type::near_ptr(Type::U8),
                    Storage::Scratch,
                ))
                .with_body(vec![
                    IrInst::LoadConst {
                        dst: Value::Local("reserved".to_owned()),
                        value: Const::U16(1),
                    },
                    IrInst::LoadAddress {
                        dst: Value::Local("ptr".to_owned()),
                        symbol: "out".to_owned(),
                        width: Width::Word,
                    },
                    IrInst::Store {
                        width: Width::Byte,
                        addr: crate::ir::Address::NearPtr(Value::Local("ptr".to_owned())),
                        src: Value::Accumulator,
                    },
                ]),
        );

        let assignment = assign_storage_with_budget(&module, 2);
        let routine = assignment.routine("scratch_pointer").unwrap();

        assert_eq!(assignment.direct_page_bytes, 4);
        assert!(matches!(
            routine.location(BindingKey::Local("reserved".to_owned())),
            Some(Location::Direct {
                offset: 0,
                size: 2,
                ..
            })
        ));
        assert!(matches!(
            routine.location(BindingKey::Local("ptr".to_owned())),
            Some(Location::Direct {
                offset: 2,
                size: 2,
                ..
            })
        ));
        validate_storage_assignment(&module, &assignment).unwrap();
    }

    #[test]
    fn keeps_three_byte_scratch_local_direct_under_budget_pressure() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("scratch_long")
                .with_local(Local::new("reserved", Type::U16, Storage::Direct))
                .with_local(Local::new("wide", Type::U24, Storage::Scratch))
                .with_body(vec![
                    IrInst::LoadConst {
                        dst: Value::Local("reserved".to_owned()),
                        value: Const::U16(1),
                    },
                    IrInst::LoadConst {
                        dst: Value::Local("wide".to_owned()),
                        value: Const::U24(0x12_3456),
                    },
                    IrInst::Store {
                        width: Width::Long,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("wide".to_owned()),
                    },
                ]),
        );

        let assignment = assign_storage_with_budget(&module, 2);
        let routine = assignment.routine("scratch_long").unwrap();

        assert_eq!(assignment.direct_page_bytes, 5);
        assert!(matches!(
            routine.location(BindingKey::Local("reserved".to_owned())),
            Some(Location::Direct {
                offset: 0,
                size: 2,
                ..
            })
        ));
        assert!(matches!(
            routine.location(BindingKey::Local("wide".to_owned())),
            Some(Location::Direct {
                offset: 2,
                size: 3,
                ..
            })
        ));
        validate_storage_assignment(&module, &assignment).unwrap();
    }

    #[test]
    fn keeps_higher_use_scratch_local_direct_under_budget_pressure() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("scratch_priority")
                .with_local(Local::new("low_use", Type::U16, Storage::Scratch))
                .with_local(Local::new("hot", Type::U16, Storage::Scratch))
                .with_body(vec![
                    IrInst::LoadConst {
                        dst: Value::Local("low_use".to_owned()),
                        value: Const::U16(1),
                    },
                    IrInst::LoadConst {
                        dst: Value::Local("hot".to_owned()),
                        value: Const::U16(2),
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("hot".to_owned()),
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("hot".to_owned()),
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("low_use".to_owned()),
                    },
                ]),
        );

        let assignment = assign_storage_with_budget(&module, 2);
        let routine = assignment.routine("scratch_priority").unwrap();

        assert_eq!(assignment.direct_page_bytes, 2);
        assert!(matches!(
            routine.location(BindingKey::Local("hot".to_owned())),
            Some(Location::Direct { symbol, .. }) if symbol == "__dp_scratch_0"
        ));
        assert!(matches!(
            routine.location(BindingKey::Local("low_use".to_owned())),
            Some(Location::Stack { offset: 1, size: 2 })
        ));
        validate_storage_assignment(&module, &assignment).unwrap();
    }

    #[test]
    fn keeps_explicit_direct_locals_direct_even_when_budget_is_exceeded() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("hard_direct")
                .with_local(Local::new("first", Type::U16, Storage::Direct))
                .with_local(Local::new("second", Type::U16, Storage::Direct))
                .with_body(vec![
                    IrInst::LoadConst {
                        dst: Value::Local("first".to_owned()),
                        value: Const::U16(1),
                    },
                    IrInst::LoadConst {
                        dst: Value::Local("second".to_owned()),
                        value: Const::U16(2),
                    },
                ]),
        );

        let assignment = assign_storage_with_budget(&module, 2);
        let routine = assignment.routine("hard_direct").unwrap();

        assert_eq!(assignment.direct_page_bytes, 4);
        assert!(matches!(
            routine.location(BindingKey::Local("first".to_owned())),
            Some(Location::Direct {
                offset: 0,
                size: 2,
                ..
            })
        ));
        assert!(matches!(
            routine.location(BindingKey::Local("second".to_owned())),
            Some(Location::Direct {
                offset: 2,
                size: 2,
                ..
            })
        ));
        validate_storage_assignment(&module, &assignment).unwrap();
    }

    #[test]
    fn keeps_call_live_scratch_locals_private() {
        let module = Module::new("layout")
            .with_routine(
                Routine::procedure("caller")
                    .with_local(Local::new("saved", Type::U16, Storage::Scratch))
                    .with_body(vec![
                        IrInst::LoadConst {
                            dst: Value::Local("saved".to_owned()),
                            value: Const::U16(1),
                        },
                        IrInst::CallProcedure {
                            name: "callee".to_owned(),
                            abi: Abi::Internal,
                        },
                        IrInst::Store {
                            width: Width::Word,
                            addr: crate::ir::Address::Direct("out".to_owned()),
                            src: Value::Local("saved".to_owned()),
                        },
                    ]),
            )
            .with_routine(Routine::procedure("callee").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Temp("tmp".to_owned()),
                    value: Const::U16(2),
                },
                IrInst::Store {
                    width: Width::Word,
                    addr: crate::ir::Address::Direct("out".to_owned()),
                    src: Value::Temp("tmp".to_owned()),
                },
            ]));

        let assignment = assign_storage(&module);
        let caller = assignment.routine("caller").unwrap();
        let callee = assignment.routine("callee").unwrap();

        assert_eq!(assignment.direct_page_bytes, 4);
        assert_ne!(
            caller.location(BindingKey::Local("saved".to_owned())),
            callee.location(BindingKey::Temp("tmp".to_owned()))
        );
        assert!(matches!(
            caller.location(BindingKey::Local("saved".to_owned())),
            Some(Location::Direct { symbol, .. }) if symbol == "__dp_caller_local_saved"
        ));
    }

    #[test]
    fn keeps_branch_live_scratch_locals_separate() {
        let done = Label::new("done");
        let module = Module::new("layout").with_routine(
            Routine::procedure("branchy")
                .with_local(Local::new("live", Type::U16, Storage::Scratch))
                .with_local(Local::new("skipped", Type::U16, Storage::Scratch))
                .with_body(vec![
                    IrInst::LoadConst {
                        dst: Value::Local("live".to_owned()),
                        value: Const::U16(1),
                    },
                    IrInst::BranchIfZero {
                        value: Value::Local("live".to_owned()),
                        target: done.clone(),
                    },
                    IrInst::LoadConst {
                        dst: Value::Local("skipped".to_owned()),
                        value: Const::U16(2),
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("skipped".to_owned()),
                    },
                    IrInst::Label(done),
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Local("live".to_owned()),
                    },
                ]),
        );

        let assignment = assign_storage(&module);
        let routine = assignment.routine("branchy").unwrap();

        assert_eq!(assignment.direct_page_bytes, 4);
        assert_ne!(
            routine.location(BindingKey::Local("live".to_owned())),
            routine.location(BindingKey::Local("skipped".to_owned()))
        );
        validate_storage_assignment(&module, &assignment).unwrap();
    }

    #[test]
    fn assigns_reentrant_scratch_locals_to_stack() {
        let mut routine = Routine::procedure("parse")
            .with_local(Local::new("scratch", Type::U16, Storage::Scratch))
            .with_body(vec![IrInst::LoadConst {
                dst: Value::Local("scratch".to_owned()),
                value: Const::U16(1),
            }]);
        routine.abi = crate::ir::Abi::Reentrant;
        let module = Module::new("layout").with_routine(routine);

        let assignment = assign_storage(&module);
        let routine = assignment.routine("parse").unwrap();

        assert!(matches!(
            routine.location(BindingKey::Local("scratch".to_owned())),
            Some(Location::Stack { offset: 1, size: 2 })
        ));
        assert_eq!(routine.stack_frame_bytes, 2);
        assert_eq!(assignment.direct_page_bytes, 0);
        validate_storage_assignment(&module, &assignment).unwrap();
    }

    #[test]
    fn rejects_incomplete_storage_assignment_for_referenced_scratch_local() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("broken")
                .with_local(Local::new("scratch", Type::U16, Storage::Scratch))
                .with_body(vec![IrInst::LoadConst {
                    dst: Value::Local("scratch".to_owned()),
                    value: Const::U16(1),
                }]),
        );
        let assignment = StorageAssignment {
            module_name: "layout".to_owned(),
            overlay: BTreeMap::new(),
            overlay_bytes: 0,
            routines: vec![RoutineStorage {
                name: "broken".to_owned(),
                direct_slots: Vec::new(),
                static_locals: Vec::new(),
                stack_slots: Vec::new(),
                stack_frame_bytes: 0,
                zeroed_registers: Vec::new(),
                bindings: BTreeMap::new(),
            }],
            direct_page_bytes: 0,
        };

        let error = validate_storage_assignment(&module, &assignment).unwrap_err();

        assert_eq!(
            error.missing[0].key,
            BindingKey::Local("scratch".to_owned())
        );
        assert!(error
            .to_string()
            .contains("routine broken is missing storage for Local(\"scratch\")"));
    }

    #[test]
    fn reuses_volatile_direct_param_slots_across_internal_routines() {
        let module = Module::new("layout")
            .with_routine(
                Routine::procedure("first")
                    .with_param(crate::ir::Param::new("value", Type::U16).direct())
                    .with_body(vec![IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Param("value".to_owned()),
                    }]),
            )
            .with_routine(
                Routine::procedure("second")
                    .with_param(crate::ir::Param::new("value", Type::U16).direct())
                    .with_body(vec![IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Param("value".to_owned()),
                    }]),
            );

        let assignment = assign_storage(&module);
        let first = assignment.routine("first").unwrap();
        let second = assignment.routine("second").unwrap();

        assert_eq!(assignment.direct_page_bytes, 2);
        assert_eq!(
            first.location(BindingKey::Param("value".to_owned())),
            second.location(BindingKey::Param("value".to_owned()))
        );
        assert!(matches!(
            first.location(BindingKey::Param("value".to_owned())),
            Some(Location::Direct { symbol, .. }) if symbol == "__dp_arg_0"
        ));
    }

    #[test]
    fn reuses_volatile_direct_param_slots_for_multi_param_internal_routines() {
        let module = Module::new("layout")
            .with_routine(
                Routine::procedure("first")
                    .with_param(crate::ir::Param::new("left", Type::U8).direct())
                    .with_param(crate::ir::Param::new("right", Type::U16).direct())
                    .with_body(vec![
                        IrInst::Store {
                            width: Width::Byte,
                            addr: crate::ir::Address::Direct("out".to_owned()),
                            src: Value::Param("left".to_owned()),
                        },
                        IrInst::Store {
                            width: Width::Word,
                            addr: crate::ir::Address::Direct("out".to_owned()),
                            src: Value::Param("right".to_owned()),
                        },
                    ]),
            )
            .with_routine(
                Routine::procedure("second")
                    .with_param(crate::ir::Param::new("left", Type::U8).direct())
                    .with_param(crate::ir::Param::new("right", Type::U16).direct())
                    .with_body(vec![
                        IrInst::Store {
                            width: Width::Byte,
                            addr: crate::ir::Address::Direct("out".to_owned()),
                            src: Value::Param("left".to_owned()),
                        },
                        IrInst::Store {
                            width: Width::Word,
                            addr: crate::ir::Address::Direct("out".to_owned()),
                            src: Value::Param("right".to_owned()),
                        },
                    ]),
            );

        let assignment = assign_storage(&module);
        let first = assignment.routine("first").unwrap();
        let second = assignment.routine("second").unwrap();

        assert_eq!(assignment.direct_page_bytes, 3);
        assert_eq!(
            first.location(BindingKey::Param("left".to_owned())),
            second.location(BindingKey::Param("left".to_owned()))
        );
        assert_eq!(
            first.location(BindingKey::Param("right".to_owned())),
            second.location(BindingKey::Param("right".to_owned()))
        );
        assert_ne!(
            first.location(BindingKey::Param("left".to_owned())),
            first.location(BindingKey::Param("right".to_owned()))
        );
        assert!(matches!(
            first.location(BindingKey::Param("left".to_owned())),
            Some(Location::Direct { symbol, size: 1, .. }) if symbol == "__dp_arg_0"
        ));
        assert!(matches!(
            first.location(BindingKey::Param("right".to_owned())),
            Some(Location::Direct { symbol, size: 2, .. }) if symbol == "__dp_arg_1"
        ));
    }

    #[test]
    fn rewrites_direct_call_argument_symbols_to_shared_param_slots() {
        let module = Module::new("layout")
            .with_routine(Routine::procedure("caller").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Direct(crate::ir::param_symbol("callee", "value")),
                    value: Const::U16(7),
                },
                IrInst::CallProcedure {
                    name: "callee".to_owned(),
                    abi: Abi::Internal,
                },
            ]))
            .with_routine(
                Routine::procedure("callee")
                    .with_param(crate::ir::Param::new("value", Type::U16).direct())
                    .with_body(vec![IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Param("value".to_owned()),
                    }]),
            );

        let assignment = assign_storage(&module);
        let rewritten = apply_storage(&module, &assignment);

        assert_eq!(
            rewritten.routines[0].body[0],
            IrInst::LoadConst {
                dst: Value::Direct("__dp_arg_0".to_owned()),
                value: Const::U16(7),
            }
        );
    }

    #[test]
    fn rewrites_multi_param_call_argument_symbols_to_shared_param_slots() {
        let module = Module::new("layout")
            .with_routine(Routine::procedure("caller").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Direct(crate::ir::param_symbol("callee", "left")),
                    value: Const::U8(7),
                },
                IrInst::LoadConst {
                    dst: Value::Direct(crate::ir::param_symbol("callee", "right")),
                    value: Const::U16(0x1234),
                },
                IrInst::CallProcedure {
                    name: "callee".to_owned(),
                    abi: Abi::Internal,
                },
            ]))
            .with_routine(
                Routine::procedure("callee")
                    .with_param(crate::ir::Param::new("left", Type::U8).direct())
                    .with_param(crate::ir::Param::new("right", Type::U16).direct())
                    .with_body(vec![
                        IrInst::Store {
                            width: Width::Byte,
                            addr: crate::ir::Address::Direct("out".to_owned()),
                            src: Value::Param("left".to_owned()),
                        },
                        IrInst::Store {
                            width: Width::Word,
                            addr: crate::ir::Address::Direct("out".to_owned()),
                            src: Value::Param("right".to_owned()),
                        },
                    ]),
            );

        let assignment = assign_storage(&module);
        let rewritten = apply_storage(&module, &assignment);

        assert_eq!(
            rewritten.routines[0].body[0],
            IrInst::LoadConst {
                dst: Value::Direct("__dp_arg_0".to_owned()),
                value: Const::U8(7),
            }
        );
        assert_eq!(
            rewritten.routines[0].body[1],
            IrInst::LoadConst {
                dst: Value::Direct("__dp_arg_1".to_owned()),
                value: Const::U16(0x1234),
            }
        );
    }

    #[test]
    fn keeps_call_live_direct_params_private() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("caller")
                .with_param(crate::ir::Param::new("value", Type::U16).direct())
                .with_body(vec![
                    IrInst::Compare {
                        width: Width::Word,
                        signed: false,
                        lhs: Value::Param("value".to_owned()),
                        rhs: Value::Accumulator,
                    },
                    IrInst::CallProcedure {
                        name: "callee".to_owned(),
                        abi: Abi::Internal,
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Param("value".to_owned()),
                    },
                ]),
        );

        let assignment = assign_storage(&module);
        let caller = assignment.routine("caller").unwrap();

        assert_eq!(assignment.direct_page_bytes, 2);
        assert!(matches!(
            caller.location(BindingKey::Param("value".to_owned())),
            Some(Location::Direct { symbol, .. }) if symbol == "__dp_caller_param_value"
        ));
    }

    #[test]
    fn only_call_live_params_get_private_direct_slots() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("mixed")
                .with_param(crate::ir::Param::new("short", Type::U8).direct())
                .with_param(crate::ir::Param::new("saved", Type::U16).direct())
                .with_body(vec![
                    IrInst::Store {
                        width: Width::Byte,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Param("short".to_owned()),
                    },
                    IrInst::Compare {
                        width: Width::Word,
                        signed: false,
                        lhs: Value::Param("saved".to_owned()),
                        rhs: Value::Accumulator,
                    },
                    IrInst::CallProcedure {
                        name: "callee".to_owned(),
                        abi: Abi::Internal,
                    },
                    IrInst::Store {
                        width: Width::Word,
                        addr: crate::ir::Address::Direct("out".to_owned()),
                        src: Value::Param("saved".to_owned()),
                    },
                ]),
        );

        let assignment = assign_storage(&module);
        let mixed = assignment.routine("mixed").unwrap();

        assert_eq!(assignment.direct_page_bytes, 3);
        assert!(matches!(
            mixed.location(BindingKey::Param("short".to_owned())),
            Some(Location::Direct { symbol, size: 1, .. }) if symbol == "__dp_arg_0"
        ));
        assert!(matches!(
            mixed.location(BindingKey::Param("saved".to_owned())),
            Some(Location::Direct { symbol, size: 2, .. }) if symbol == "__dp_mixed_param_saved"
        ));
    }

    #[test]
    fn rewrites_direct_assigned_values() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("copy")
                .with_local(Local::new("count", Type::U16, Storage::Direct))
                .with_body(vec![IrInst::Dec {
                    width: Width::Word,
                    value: Value::Local("count".to_owned()),
                }]),
        );

        let assignment = assign_storage(&module);
        let rewritten = apply_storage(&module, &assignment);

        assert_eq!(
            rewritten.routines[0].body[0],
            IrInst::Dec {
                width: Width::Word,
                value: Value::Direct("__dp_copy_local_count".to_owned())
            }
        );
    }

    #[test]
    fn report_includes_direct_page_usage() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("copy")
                .with_param(crate::ir::Param::new("src", Type::near_ptr(Type::U8)).direct()),
        );

        let report = assign_storage(&module).report();

        assert!(report.contains("storage layout for module layout\n"));
        assert!(report.contains("direct page bytes: 2\n"));
        assert!(report.contains("routine copy\n"));
        assert!(report.contains("direct $00..$01 __dp_arg_0"));
    }

    #[test]
    fn validates_aggregate_direct_page_usage() {
        let first = Module::new("first").with_routine(
            Routine::procedure("copy")
                .with_param(crate::ir::Param::new("src", Type::near_ptr(Type::U8)).direct()),
        );
        let second = Module::new("second").with_routine(
            Routine::procedure("store")
                .with_param(crate::ir::Param::new("value", Type::U16).direct()),
        );

        let assignments = vec![assign_storage(&first), assign_storage(&second)];
        let usage = validate_direct_page_budget(&assignments, 4).unwrap();

        assert_eq!(usage.used, 4);
        assert_eq!(usage.remaining(), Some(0));
        assert!(usage.report().contains("direct page usage: 4 / 4 bytes"));
        assert!(usage.report().contains("  first: 2 bytes"));
        assert!(usage.report().contains("  second: 2 bytes"));
    }

    #[test]
    fn rejects_aggregate_direct_page_overflow() {
        let first = Module::new("first").with_routine(
            Routine::procedure("copy")
                .with_param(crate::ir::Param::new("src", Type::near_ptr(Type::U8)).direct()),
        );
        let second = Module::new("second").with_routine(
            Routine::procedure("store")
                .with_param(crate::ir::Param::new("value", Type::U16).direct()),
        );

        let assignments = vec![assign_storage(&first), assign_storage(&second)];
        let error = validate_direct_page_budget(&assignments, 3).unwrap_err();

        assert_eq!(error.overage(), 1);
        assert!(error
            .to_string()
            .contains("direct page allocation exceeds budget by 1 bytes"));
        assert!(error
            .to_string()
            .contains("direct page usage: 4 / 3 bytes (1 over)"));
    }

    fn register_options() -> StorageOptions {
        StorageOptions {
            wide_scratch_direct: false,
            temp_registers: Some(("__tmp_regs", 8)),
            cheap_frames: false,
        }
    }

    #[test]
    fn reentrant_locals_live_across_a_loop_call_stay_in_the_frame() {
        let module = crate::frontend::compile_source(
            "
            module Loops
            procedure tick()
                return
            end
            export reentrant function count(limit: u16): u16
                var total: u16
                var i: u16
                total = 0
                i = 0
                while i != limit
                    tick()
                    i = i + 1
                end
                return total
            end
            ",
        )
        .unwrap();
        let assignment = assign_storage_with_options(&module, None, register_options());
        let layout = assignment.routine("count").unwrap();
        // `i` is live around the loop, which calls `tick`.
        assert!(matches!(
            layout.location(BindingKey::Local("i".to_owned())),
            Some(Location::Stack { .. })
        ));
    }

    #[test]
    fn reentrant_loop_locals_without_calls_use_temp_registers_and_start_at_zero() {
        let module = crate::frontend::compile_source(
            "
            module Loops
            procedure tick()
                return
            end
            export reentrant function count(limit: u16): u16
                var i: u16
                i = 0
                while i != limit
                    i = i + 1
                end
                tick()
                return limit
            end
            ",
        )
        .unwrap();
        let assignment = assign_storage_with_options(&module, None, register_options());
        let layout = assignment.routine("count").unwrap();
        assert!(matches!(
            layout.location(BindingKey::Local("i".to_owned())),
            Some(Location::Direct { symbol, .. }) if symbol.starts_with("__tmp_regs")
        ));
        let applied = apply_storage(&module, &assignment);
        let count = applied
            .routines
            .iter()
            .find(|routine| routine.name == "count")
            .unwrap();
        assert!(matches!(
            &count.body[0],
            IrInst::LoadConst { dst: Value::Direct(symbol), value: Const::U16(0) }
                if symbol.starts_with("__tmp_regs")
        ));
    }

    #[test]
    fn overlays_private_slots_of_routines_that_never_run_together() {
        let module = crate::frontend::compile_source(
            "
            module Overlay
            external procedure elsewhere()
            procedure tick()
                return
            end
            # Each keeps a parameter alive across a call: a private slot.
            procedure left(a: u16)
                tick()
                a = a + 1
                return
            end
            procedure right(b: u16)
                tick()
                b = b + 1
                return
            end
            procedure outer(c: u16)
                left(c)
                c = c + 1
                return
            end
            export procedure callback(d: u16)
                tick()
                d = d + 1
                return
            end
            procedure leaves(e: u16)
                elsewhere()
                e = e + 1
                return
            end
            ",
        )
        .unwrap();
        let assignment = assign_storage(&module);
        let at = |name: &str| assignment.overlay.get(name).copied();
        // left and right never run together; outer calls left.
        assert_eq!(at("__dp_left_param_a"), at("__dp_right_param_b"));
        assert_ne!(at("__dp_outer_param_c"), at("__dp_left_param_a"));
        // `elsewhere` might call back into the exported routine.
        assert_ne!(at("__dp_leaves_param_e"), at("__dp_callback_param_d"));
        assert!(at("__dp_left_param_a").is_some());
    }

    #[test]
    fn pointer_locals_live_in_the_direct_page() {
        let module = crate::frontend::compile_source(
            "
            module Pointers
            procedure walk()
                var far_cell: far ^u8
                var near_cell: near ^u8
                var count: u16
                far_cell = $40000
                near_cell = $2000
                count = 0
                while count != 10
                    far_cell^ = near_cell^
                    count = count + 1
                end
                return
            end
            ",
        )
        .unwrap();
        let assignment = assign_storage(&module);
        let walk = assignment
            .routines
            .iter()
            .find(|layout| layout.name == "walk")
            .unwrap();
        let place = |name: &str| walk.bindings.get(&BindingKey::Local(name.to_owned())).cloned();
        assert!(matches!(place("far_cell"), Some(Location::Direct { .. })));
        assert!(matches!(place("near_cell"), Some(Location::Direct { .. })));
        assert!(matches!(place("count"), Some(Location::Static { .. })));
    }

    #[test]
    fn calls_out_do_not_come_back_into_the_programs_main() {
        let module = crate::frontend::compile_source(
            "
            module Program
            external procedure elsewhere()
            procedure first(a: u16)
                elsewhere()
                a = a + 1
                return
            end
            procedure second(b: u16)
                elsewhere()
                b = b + 1
                return
            end
            export function main(): u8
                first(1)
                second(2)
                return 0
            end
            ",
        )
        .unwrap();
        let assignment = assign_storage(&module);
        let at = |name: &str| assignment.overlay.get(name).copied();
        assert!(at("__dp_first_param_a").is_some());
        assert_eq!(at("__dp_first_param_a"), at("__dp_second_param_b"));
    }
}
