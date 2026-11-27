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
    pub external_data: Vec<ExternalData>,
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
            external_data: Vec::new(),
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
        out.static_data = module.static_data.clone();
        for global in &module.globals {
            let size = global.ty.size_bytes();
            if let (Some(items), GlobalPlacement::Static) = (&global.init, &global.placement) {
                out.static_data.push(StaticData {
                    name: global.name.clone(),
                    items: items.clone(),
                    export: global.export,
                });
                continue;
            }
            match global.placement {
                GlobalPlacement::Static => out.static_symbols.push(StaticSymbol {
                    name: global.name.clone(),
                    size,
                    export: global.export,
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
        out.external_data = module.external_data.clone();
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
            // Callers store the parameters of exported routines, except
            // `stack` routines, which copy their own in from the stack.
            .map(|routine| {
                (
                    routine.name.as_str(),
                    routine.export && !routine.abi.stack_arguments(),
                )
            })
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
        let mut symbols = Vec::new();
        if assignment.overlay_bytes != 0 {
            symbols.push(DirectSymbol {
                name: crate::storage::OVERLAY_SYMBOL.to_owned(),
                size: assignment.overlay_bytes,
                export: false,
                alias: None,
            });
        }
        symbols.extend(direct.into_iter().map(|(name, (size, export))| {
            let alias = assignment
                .overlay
                .get(&name)
                .map(|offset| (crate::storage::OVERLAY_SYMBOL.to_owned(), *offset));
            DirectSymbol {
                name,
                size,
                export,
                alias,
            }
        }));
        self.direct_symbols = symbols;

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
        self.static_symbols
            .extend(statics.into_iter().map(|(name, size)| StaticSymbol {
                name,
                size,
                export: false,
            }));
    }
}

pub use crate::ir::{DataItem, ExternalData, StaticData};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectSymbol {
    pub name: String,
    pub size: u16,
    pub export: bool,
    /// Defined as `base + offset` rather than reserved (an overlay slot).
    pub alias: Option<(String, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticSymbol {
    pub name: String,
    pub size: u16,
    pub export: bool,
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
    /// Segment for this routine, when not the module's code segment.
    pub segment: Option<String>,
    pub insts: Vec<I>,
}

impl<I> AsmRoutine<I> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            export: false,
            segment: None,
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

// ---- names ca65 would misread ------------------------------------------------

/// The symbol `name` goes by in the output: a name ca65 reads as a
/// register or an instruction (`x`, `row`, `bit`) gets a prefix, the same
/// in every module, so an exported one still meets its imports.
pub fn output_name(name: &str) -> String {
    if crate::frontend::clashes_with_assembler(name) {
        format!("_m_{name}")
    } else {
        name.to_owned()
    }
}

/// `text` (an operand: a symbol, `name+3`, `<name`, ...) with each symbol in
/// it given its output name; numbers (`$a0`, `12`) are left alone.
pub fn rename_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut after_number_sign = false;
    while let Some(ch) = chars.next() {
        if ch.is_ascii_alphabetic() || ch == '_' {
            let mut word = String::from(ch);
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    word.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if after_number_sign {
                out.push_str(&word);
            } else {
                out.push_str(&output_name(&word));
            }
            after_number_sign = false;
        } else if ch.is_ascii_digit() {
            // The rest of a number (hex digits included) is not a symbol.
            out.push(ch);
            after_number_sign = true;
        } else {
            out.push(ch);
            after_number_sign = ch == '$' || ch == '%';
        }
    }
    out
}

/// Instructions whose symbols can be renamed (inline assembly is left as
/// written).
pub trait RenameSymbols {
    fn rename_symbols(&mut self);
}

impl<I: RenameSymbols> AsmModule<I> {
    /// Gives every symbol in the module its output name (`output_name`).
    pub fn rename_clashing_symbols(&mut self) {
        for symbol in &mut self.direct_symbols {
            symbol.name = rename_tokens(&symbol.name);
            if let Some((base, _)) = &mut symbol.alias {
                *base = rename_tokens(base);
            }
        }
        for symbol in &mut self.static_symbols {
            symbol.name = rename_tokens(&symbol.name);
        }
        for data in &mut self.static_data {
            data.name = rename_tokens(&data.name);
            for item in &mut data.items {
                if let DataItem::Address { symbol, .. } = item {
                    *symbol = rename_tokens(symbol);
                }
            }
        }
        for symbol in &mut self.fixed_symbols {
            symbol.name = rename_tokens(&symbol.name);
        }
        for symbol in &mut self.banked_symbols {
            symbol.name = rename_tokens(&symbol.name);
        }
        for vector in &mut self.vectors {
            vector.routine = rename_tokens(&vector.routine);
        }
        for name in self
            .external_routines
            .iter_mut()
            .chain(self.external_direct_symbols.iter_mut())
        {
            *name = rename_tokens(name);
        }
        for data in &mut self.external_data {
            data.name = rename_tokens(&data.name);
        }
        if let Some(entry) = &mut self.entry {
            entry.routine = rename_tokens(&entry.routine);
        }
        for routine in &mut self.routines {
            routine.name = rename_tokens(&routine.name);
            for inst in &mut routine.insts {
                inst.rename_symbols();
            }
        }
    }
}
