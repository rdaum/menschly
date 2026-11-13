//! Target-neutral assembly module structure.
//!
//! Every backend lowers IR into an [`AsmModule`] parameterised by its own
//! instruction type. The module-level declarations (symbols, segments,
//! vectors, metadata) are shared, so the ca65 emitter only needs a
//! per-target way to print instructions.

use std::collections::BTreeMap;

use crate::ir::{EntryKind, GlobalPlacement, Module};
use crate::storage::{BindingKey, StorageAssignment};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmModule<I> {
    pub name: String,
    pub direct_symbols: Vec<DirectSymbol>,
    pub static_symbols: Vec<StaticSymbol>,
    pub static_data: Vec<StaticData>,
    pub fixed_symbols: Vec<FixedSymbol>,
    pub banked_symbols: Vec<BankedSymbol>,
    pub vectors: Vec<VectorEntry>,
    pub metadata: Vec<ModuleMetadata>,
    pub imports: Vec<ModuleImport>,
    pub libraries: Vec<ModuleLibrary>,
    pub external_routines: Vec<String>,
    pub external_direct_symbols: Vec<String>,
    pub entry: Option<ModuleEntry>,
    pub routines: Vec<AsmRoutine<I>>,
}

impl<I> AsmModule<I> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            direct_symbols: Vec::new(),
            static_symbols: Vec::new(),
            static_data: Vec::new(),
            fixed_symbols: Vec::new(),
            banked_symbols: Vec::new(),
            vectors: Vec::new(),
            metadata: Vec::new(),
            imports: Vec::new(),
            libraries: Vec::new(),
            external_routines: Vec::new(),
            external_direct_symbols: Vec::new(),
            entry: None,
            routines: Vec::new(),
        }
    }

    pub fn with_routine(mut self, routine: AsmRoutine<I>) -> Self {
        self.routines.push(routine);
        self
    }

    /// Builds a module carrying every declaration from `module` but no routines.
    pub fn declarations_from(module: &Module) -> Self {
        let mut out = Self::new(module.name.clone());
        for global in &module.globals {
            let size = global.ty.size_bytes();
            match global.placement {
                GlobalPlacement::Static => out.static_symbols.push(StaticSymbol {
                    name: global.name.clone(),
                    size,
                }),
                GlobalPlacement::Fixed(address) => out.fixed_symbols.push(FixedSymbol {
                    name: global.name.clone(),
                    address,
                    size,
                    export: global.export,
                }),
                GlobalPlacement::Bank(bank) => out.banked_symbols.push(BankedSymbol {
                    name: global.name.clone(),
                    bank,
                    size,
                    export: global.export,
                }),
            }
        }
        out.static_data = module
            .static_data
            .iter()
            .map(|data| StaticData {
                name: data.name.clone(),
                bytes: data.bytes.clone(),
            })
            .collect();
        out.vectors = module
            .vectors
            .iter()
            .map(|vector| VectorEntry {
                name: vector.name.clone(),
                routine: vector.routine.clone(),
            })
            .collect();
        out.metadata = module
            .metadata
            .iter()
            .map(|metadata| ModuleMetadata {
                key: metadata.key.clone(),
                value: metadata.value.clone(),
            })
            .collect();
        out.imports = module
            .imports
            .iter()
            .map(|import| ModuleImport {
                module: import.module.clone(),
                alias: import.alias.clone(),
            })
            .collect();
        out.libraries = module
            .libraries
            .iter()
            .map(|library| ModuleLibrary {
                name: library.name.clone(),
            })
            .collect();
        out.external_routines = module.external_routines.clone();
        out.external_direct_symbols = module.external_direct_symbols.clone();
        out.entry = module.entry.as_ref().map(|entry| ModuleEntry {
            kind: match entry.kind {
                EntryKind::Application => "application",
                EntryKind::Command => "command",
            }
            .to_owned(),
            routine: entry.routine.clone(),
        });
        out
    }

    /// Adds the direct-page slots and static locals chosen by the storage pass.
    ///
    /// `assigned` is the module after [`crate::storage::apply_storage`]; it is
    /// used to decide which parameter slots belong to exported routines.
    pub fn add_storage_symbols(&mut self, assigned: &Module, assignment: &StorageAssignment) {
        let routine_exports = assigned
            .routines
            .iter()
            .map(|routine| (routine.name.as_str(), routine.export))
            .collect::<BTreeMap<_, _>>();
        let mut direct = BTreeMap::<String, (u16, bool)>::new();
        for routine in &assignment.routines {
            let routine_export = routine_exports
                .get(routine.name.as_str())
                .copied()
                .unwrap_or_default();
            for slot in &routine.direct_slots {
                let export = routine_export && matches!(slot.source, BindingKey::Param(_));
                direct
                    .entry(slot.symbol.clone())
                    .and_modify(|(size, existing_export)| {
                        *size = (*size).max(slot.size);
                        *existing_export |= export;
                    })
                    .or_insert((slot.size, export));
            }
        }
        self.direct_symbols = direct
            .into_iter()
            .map(|(name, (size, export))| DirectSymbol { name, size, export })
            .collect();

        let mut statics = BTreeMap::<String, u16>::new();
        for local in assignment
            .routines
            .iter()
            .flat_map(|routine| routine.static_locals.iter())
        {
            statics
                .entry(local.symbol.clone())
                .and_modify(|size| *size = (*size).max(local.size))
                .or_insert(local.size);
        }
        self.static_symbols.extend(
            statics
                .into_iter()
                .map(|(name, size)| StaticSymbol { name, size }),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticData {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectSymbol {
    pub name: String,
    pub size: u16,
    pub export: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticSymbol {
    pub name: String,
    pub size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedSymbol {
    pub name: String,
    pub address: u32,
    pub size: u16,
    pub export: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankedSymbol {
    pub name: String,
    pub bank: u8,
    pub size: u16,
    pub export: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorEntry {
    pub name: String,
    pub routine: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleMetadata {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleImport {
    pub module: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleLibrary {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleEntry {
    pub kind: String,
    pub routine: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmRoutine<I> {
    pub name: String,
    pub export: bool,
    pub insts: Vec<I>,
}

impl<I> AsmRoutine<I> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            export: false,
            insts: Vec::new(),
        }
    }

    pub fn exported(mut self) -> Self {
        self.export = true;
        self
    }

    pub fn with_inst(mut self, inst: I) -> Self {
        self.insts.push(inst);
        self
    }

    pub fn with_insts(mut self, insts: Vec<I>) -> Self {
        self.insts = insts;
        self
    }
}
