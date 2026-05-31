use std::collections::BTreeMap;
use std::fmt;

use crate::ir::{
    param_symbol, Abi, Address, Const, IrInst, Local, Module, Param, Routine, Storage, Value, Width,
};

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

pub fn assign_storage(module: &Module) -> StorageAssignment {
    let mut allocator = DirectAllocator::default();
    let routines = module
        .routines
        .iter()
        .map(|routine| assign_routine(routine, &mut allocator))
        .collect();

    StorageAssignment {
        module_name: module.name.clone(),
        routines,
        direct_page_bytes: allocator.next_offset,
    }
}

pub fn apply_storage(module: &Module, assignment: &StorageAssignment) -> Module {
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
                .map(|inst| rewrite_inst(inst, layout))
                .collect();
            routine.stack_frame_bytes = layout.stack_frame_bytes;
            routine
        })
        .collect();

    Module {
        name: module.name.clone(),
        globals: module.globals.clone(),
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

fn assign_routine(routine: &Routine, allocator: &mut DirectAllocator) -> RoutineStorage {
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
        assign_stack_params(routine, &mut out);
    } else {
        assign_params(routine, allocator, &mut out);
        assign_locals(routine, allocator, &mut out);
    }
    assign_scratch_temps(routine, allocator, &mut out);

    out
}

fn assign_params(routine: &Routine, allocator: &mut DirectAllocator, out: &mut RoutineStorage) {
    for param in &routine.params {
        allocate_direct_param(routine, param, allocator, out);
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
    out: &mut RoutineStorage,
) {
    let mut lifetimes = temp_lifetimes(routine);
    lifetimes.sort_by_key(|temp| (temp.first_use, temp.last_use, temp.name.clone()));

    let mut active: Vec<ActiveTemp> = Vec::new();
    let mut free: Vec<FreeSlot> = Vec::new();

    for temp in lifetimes {
        let mut i = 0;
        while i < active.len() {
            if active[i].last_use < temp.first_use {
                let old = active.remove(i);
                free.push(FreeSlot {
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
            None => allocator.alloc(temp_symbol(&routine.name, &temp.name), temp.size),
        };

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
        active.push(ActiveTemp {
            symbol: slot.symbol,
            offset: slot.offset,
            size: slot.size,
            last_use: temp.last_use,
        });
    }
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

fn rewrite_inst(inst: &IrInst, layout: &RoutineStorage) -> IrInst {
    match inst {
        IrInst::InlineAsm(_) => inst.clone(),
        IrInst::Move { dst, src } => IrInst::Move {
            dst: rewrite_value(dst, layout),
            src: rewrite_value(src, layout),
        },
        IrInst::LoadConst { dst, value } => IrInst::LoadConst {
            dst: rewrite_value(dst, layout),
            value: *value,
        },
        IrInst::Load { width, dst, addr } => IrInst::Load {
            width: *width,
            dst: rewrite_value(dst, layout),
            addr: rewrite_address(addr, layout),
        },
        IrInst::Store { width, addr, src } => IrInst::Store {
            width: *width,
            addr: rewrite_address(addr, layout),
            src: rewrite_value(src, layout),
        },
        IrInst::Add {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::Add {
            width: *width,
            dst: rewrite_value(dst, layout),
            lhs: rewrite_value(lhs, layout),
            rhs: rewrite_value(rhs, layout),
        },
        IrInst::Sub {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::Sub {
            width: *width,
            dst: rewrite_value(dst, layout),
            lhs: rewrite_value(lhs, layout),
            rhs: rewrite_value(rhs, layout),
        },
        IrInst::BitAnd {
            width,
            dst,
            lhs,
            rhs,
        } => IrInst::BitAnd {
            width: *width,
            dst: rewrite_value(dst, layout),
            lhs: rewrite_value(lhs, layout),
            rhs: rewrite_value(rhs, layout),
        },
        IrInst::Dec { width, value } => IrInst::Dec {
            width: *width,
            value: rewrite_value(value, layout),
        },
        IrInst::Inc { width, value } => IrInst::Inc {
            width: *width,
            value: rewrite_value(value, layout),
        },
        IrInst::Compare { width, lhs, rhs } => IrInst::Compare {
            width: *width,
            lhs: rewrite_value(lhs, layout),
            rhs: rewrite_value(rhs, layout),
        },
        IrInst::BranchIfZero { value, target } => IrInst::BranchIfZero {
            value: rewrite_value(value, layout),
            target: target.clone(),
        },
        IrInst::Push { width, value } => IrInst::Push {
            width: *width,
            value: rewrite_value(value, layout),
        },
        IrInst::CallFunction { name, dst } => IrInst::CallFunction {
            name: name.clone(),
            dst: rewrite_value(dst, layout),
        },
        IrInst::ReturnValue(value) => IrInst::ReturnValue(rewrite_value(value, layout)),
        other => other.clone(),
    }
}

fn rewrite_address(addr: &Address, layout: &RoutineStorage) -> Address {
    match addr {
        Address::NearPtr(value) => Address::NearPtr(rewrite_value(value, layout)),
        Address::FarPtr(value) => Address::FarPtr(rewrite_value(value, layout)),
        other => other.clone(),
    }
}

fn rewrite_value(value: &Value, layout: &RoutineStorage) -> Value {
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

fn temp_lifetimes(routine: &Routine) -> Vec<TempLifetime> {
    let mut temps = BTreeMap::<String, (usize, usize, u16)>::new();

    for (index, inst) in routine.body.iter().enumerate() {
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
        })
        .collect()
}

fn temp_uses(inst: &IrInst) -> Vec<(String, u16)> {
    let mut out = Vec::new();

    match inst {
        IrInst::LoadConst { dst, value } => collect_temp_value(dst, const_size(*value), &mut out),
        IrInst::Move { dst, src } => {
            collect_temp_value(dst, 2, &mut out);
            collect_temp_value(src, 2, &mut out);
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
        IrInst::Label(_)
        | IrInst::InlineAsm(_)
        | IrInst::Branch { .. }
        | IrInst::BranchIfEqual { .. }
        | IrInst::BranchIfNotEqual { .. }
        | IrInst::CallProcedure { .. }
        | IrInst::CleanupStack { .. }
        | IrInst::Return => {}
    }

    out
}

fn collect_temp_address(addr: &Address, out: &mut Vec<(String, u16)>) {
    match addr {
        Address::NearPtr(value) => collect_temp_value(value, 2, out),
        Address::FarPtr(value) => collect_temp_value(value, 3, out),
        _ => {}
    }
}

fn collect_temp_value(value: &Value, size: u16, out: &mut Vec<(String, u16)>) {
    if let Value::Temp(name) = value {
        out.push((name.clone(), size));
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
struct TempLifetime {
    name: String,
    first_use: usize,
    last_use: usize,
    size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTemp {
    symbol: String,
    offset: u16,
    size: u16,
    last_use: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FreeSlot {
    symbol: String,
    offset: u16,
    size: u16,
}

#[derive(Default)]
struct DirectAllocator {
    next_offset: u16,
}

impl DirectAllocator {
    fn alloc(&mut self, symbol: String, size: u16) -> FreeSlot {
        let slot = FreeSlot {
            symbol,
            offset: self.next_offset,
            size,
        };
        self.next_offset += size;
        slot
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

    use super::{apply_storage, assign_storage, BindingKey, Location};

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
        assert!(report.contains("direct $00..$01 __dp_copy_param_src"));
    }
}
