use std::collections::BTreeMap;
use std::fmt;

use crate::ir::{
    param_symbol, Abi, Address, Const, IrInst, Local, Module, Param, Routine, Storage, Value, Width,
};

pub const DEFAULT_DIRECT_PAGE_BUDGET: u16 = 0x80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageAssignment {
    pub module_name: String,
    pub routines: Vec<RoutineStorage>,
    pub direct_page_bytes: u16,
}

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
pub struct RoutineStorage {
    pub name: String,
    pub direct_slots: Vec<DirectSlot>,
    pub static_locals: Vec<StaticLocal>,
    pub stack_slots: Vec<StackSlot>,
    pub stack_frame_bytes: u16,
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
    let mut allocator = DirectAllocator::default();
    let mut args = DirectSlotPool::new("__dp_arg");
    let mut scratch = DirectSlotPool::new("__dp_scratch");
    let routines = module
        .routines
        .iter()
        .map(|routine| assign_routine(routine, &mut allocator, &mut args, &mut scratch))
        .collect();

    StorageAssignment {
        module_name: module.name.clone(),
        routines,
        direct_page_bytes: allocator.next_offset,
    }
}

pub fn apply_storage(module: &Module, assignment: &StorageAssignment) -> Module {
    let direct_aliases = direct_symbol_aliases(assignment);
    let routines = module
        .routines
        .iter()
        .map(|routine| {
            let Some(layout) = assignment.routine(&routine.name) else {
                return routine.clone();
            };

            let mut routine = routine.clone();
            routine.body = routine
                .body
                .iter()
                .map(|inst| rewrite_inst(inst, layout, &direct_aliases))
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
        bindings: BTreeMap::new(),
    };

    if matches!(routine.abi, Abi::Reentrant) {
        assign_locals(routine, allocator, &mut out);
        assign_scratch_temps(routine, allocator, scratch, &mut out);
        assign_stack_params(routine, &mut out);
    } else {
        assign_params(routine, allocator, args, &mut out);
        assign_locals(routine, allocator, &mut out);
        assign_scratch_temps(routine, allocator, scratch, &mut out);
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
    let can_share_params = !routine.export && routine.params.len() <= 1;
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

fn assign_locals(routine: &Routine, allocator: &mut DirectAllocator, out: &mut RoutineStorage) {
    let mut stack_offset = 1;

    for local in &routine.locals {
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

fn assign_scratch_temps(
    routine: &Routine,
    allocator: &mut DirectAllocator,
    scratch: &mut DirectSlotPool,
    out: &mut RoutineStorage,
) {
    let mut lifetimes = temp_lifetimes(routine);
    lifetimes.sort_by_key(|temp| (temp.first_use, temp.last_use, temp.name.clone()));

    if matches!(routine.abi, Abi::Reentrant) {
        for temp in lifetimes {
            let offset = out.stack_frame_bytes + 1;
            out.bindings.insert(
                BindingKey::Temp(temp.name.clone()),
                Location::Stack {
                    offset,
                    size: temp.size,
                },
            );
            out.stack_slots.push(StackSlot {
                name: temp_symbol(&routine.name, &temp.name),
                offset,
                size: temp.size,
            });
            out.stack_frame_bytes += temp.size;
        }
        return;
    }

    let mut volatile_lifetimes = Vec::new();
    let mut private_lifetimes = Vec::new();
    for temp in lifetimes {
        if temp.crosses_call {
            private_lifetimes.push(temp);
        } else {
            volatile_lifetimes.push(temp);
        }
    }

    assign_volatile_scratch_temps(volatile_lifetimes, allocator, scratch, out);
    assign_private_scratch_temps(&routine.name, private_lifetimes, allocator, out);
}

fn assign_volatile_scratch_temps(
    lifetimes: Vec<TempLifetime>,
    allocator: &mut DirectAllocator,
    scratch: &mut DirectSlotPool,
    out: &mut RoutineStorage,
) {
    let mut active: Vec<ActiveDirectSlot> = Vec::new();
    let mut free = scratch.slots.clone();

    for temp in lifetimes {
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
            None => scratch.alloc(allocator, temp.size),
        };

        bind_direct_temp(out, &temp, &slot);
        active.push(ActiveDirectSlot {
            symbol: slot.symbol,
            offset: slot.offset,
            size: slot.size,
            last_use: temp.last_use,
        });
    }
}

fn assign_private_scratch_temps(
    routine_name: &str,
    lifetimes: Vec<TempLifetime>,
    allocator: &mut DirectAllocator,
    out: &mut RoutineStorage,
) {
    let mut active: Vec<ActiveDirectSlot> = Vec::new();
    let mut free: Vec<AllocatedSlot> = Vec::new();

    for temp in lifetimes {
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
            None => allocator.alloc(temp_symbol(routine_name, &temp.name), temp.size),
        };

        bind_direct_temp(out, &temp, &slot);
        active.push(ActiveDirectSlot {
            symbol: slot.symbol,
            offset: slot.offset,
            size: slot.size,
            last_use: temp.last_use,
        });
    }
}

fn bind_direct_temp(out: &mut RoutineStorage, temp: &TempLifetime, slot: &AllocatedSlot) {
    out.bindings.insert(
        BindingKey::Temp(temp.name.clone()),
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
        source: BindingKey::Temp(temp.name.clone()),
    });
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
        IrInst::Dec { width, value } => IrInst::Dec {
            width: *width,
            value: rewrite_value(value, layout, direct_aliases),
        },
        IrInst::Inc { width, value } => IrInst::Inc {
            width: *width,
            value: rewrite_value(value, layout, direct_aliases),
        },
        IrInst::Compare { width, lhs, rhs } => IrInst::Compare {
            width: *width,
            lhs: rewrite_value(lhs, layout, direct_aliases),
            rhs: rewrite_value(rhs, layout, direct_aliases),
        },
        IrInst::BranchIfZero { value, target } => IrInst::BranchIfZero {
            value: rewrite_value(value, layout, direct_aliases),
            target: target.clone(),
        },
        IrInst::Push { width, value } => IrInst::Push {
            width: *width,
            value: rewrite_value(value, layout, direct_aliases),
        },
        IrInst::CallFunction { name, dst } => IrInst::CallFunction {
            name: name.clone(),
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        IrInst::CopSignature { dst } => IrInst::CopSignature {
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        IrInst::CopRegister { register, dst } => IrInst::CopRegister {
            register: *register,
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        IrInst::CopCall {
            family,
            call,
            x,
            y,
            dst,
        } => IrInst::CopCall {
            family: *family,
            call: rewrite_value(call, layout, direct_aliases),
            x: rewrite_value(x, layout, direct_aliases),
            y: rewrite_value(y, layout, direct_aliases),
            dst: rewrite_value(dst, layout, direct_aliases),
        },
        IrInst::ReturnValue(value) => {
            IrInst::ReturnValue(rewrite_value(value, layout, direct_aliases))
        }
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
            let crosses_call = match (first_use, last_use) {
                (Some(first_use), Some(last_use)) => call_indices
                    .iter()
                    .any(|call| first_use < *call && *call < last_use),
                _ => false,
            };
            ParamLifetime { name, crosses_call }
        })
        .collect()
}

fn temp_lifetimes(routine: &Routine) -> Vec<TempLifetime> {
    let mut temps = BTreeMap::<String, (usize, usize, u16)>::new();
    let mut call_indices = Vec::new();

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
                })
                .or_insert((index, index, size));
        }
    }

    temps
        .into_iter()
        .map(|(name, (first_use, last_use, size))| TempLifetime {
            name,
            first_use,
            last_use,
            size,
            crosses_call: call_indices
                .iter()
                .any(|call| first_use < *call && *call < last_use),
        })
        .collect()
}

fn is_call_barrier(inst: &IrInst) -> bool {
    matches!(
        inst,
        IrInst::CallProcedure { .. } | IrInst::CallFunction { .. } | IrInst::CopCall { .. }
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
        } => {
            collect_param_value(dst, &mut out);
            collect_param_value(lhs, &mut out);
            collect_param_value(rhs, &mut out);
        }
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
        IrInst::CopCall {
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
        } => {
            let size = width.size_bytes();
            collect_temp_value(dst, size, &mut out);
            collect_temp_value(lhs, size, &mut out);
            collect_temp_value(rhs, size, &mut out);
        }
        IrInst::Dec { width, value } | IrInst::Inc { width, value } => {
            collect_temp_value(value, width.size_bytes(), &mut out);
        }
        IrInst::Compare { width, lhs, rhs } => {
            let size = width.size_bytes();
            collect_temp_value(lhs, size, &mut out);
            collect_temp_value(rhs, size, &mut out);
        }
        IrInst::BranchIfZero { value, .. } | IrInst::ReturnValue(value) => {
            collect_temp_value(value, 2, &mut out);
        }
        IrInst::Push { width, value } => collect_temp_value(value, width.size_bytes(), &mut out),
        IrInst::CallFunction { dst, .. } => collect_temp_value(dst, 2, &mut out),
        IrInst::CopSignature { dst } => collect_temp_value(dst, 1, &mut out),
        IrInst::CopRegister { dst, .. } => collect_temp_value(dst, 2, &mut out),
        IrInst::CopCall {
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

fn collect_temp_address(addr: &Address, out: &mut Vec<(String, u16)>) {
    match addr {
        Address::ValueOffset { base, .. } => collect_temp_value(base, 2, out),
        Address::NearPtr(value) => collect_temp_value(value, 2, out),
        Address::FarPtr(value) => collect_temp_value(value, 3, out),
        _ => {}
    }
}

fn collect_param_address(addr: &Address, out: &mut Vec<String>) {
    match addr {
        Address::ValueOffset { base, .. } => collect_param_value(base, out),
        Address::NearPtr(value) => collect_param_value(value, out),
        Address::FarPtr(value) => collect_param_value(value, out),
        _ => {}
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
    match value {
        Const::U8(_) => 1,
        Const::U16(_) => 2,
        Const::U24(_) => 3,
    }
}

impl Width {
    fn size_bytes(self) -> u16 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Long => 3,
        }
    }
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

#[derive(Default)]
struct DirectAllocator {
    next_offset: u16,
}

impl DirectAllocator {
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
    prefix: &'static str,
    slots: Vec<AllocatedSlot>,
}

impl DirectSlotPool {
    fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
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
    use crate::ir::{Const, IrInst, Local, Module, Routine, Storage, Type, Value, Width};

    use super::{apply_storage, assign_storage, validate_direct_page_budget, BindingKey, Location};

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
    fn rewrites_direct_call_argument_symbols_to_shared_param_slots() {
        let module = Module::new("layout")
            .with_routine(Routine::procedure("caller").with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Direct(crate::ir::param_symbol("callee", "value")),
                    value: Const::U16(7),
                },
                IrInst::CallProcedure {
                    name: "callee".to_owned(),
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
    fn keeps_call_live_direct_params_private() {
        let module = Module::new("layout").with_routine(
            Routine::procedure("caller")
                .with_param(crate::ir::Param::new("value", Type::U16).direct())
                .with_body(vec![
                    IrInst::Compare {
                        width: Width::Word,
                        lhs: Value::Param("value".to_owned()),
                        rhs: Value::Accumulator,
                    },
                    IrInst::CallProcedure {
                        name: "callee".to_owned(),
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
}
