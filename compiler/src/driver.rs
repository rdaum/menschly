//! One-call compilation from Mensch source (or IR) to ca65 assembly.

use std::fmt;
use std::str::FromStr;

use crate::ir::Module;
use crate::storage::StorageAssignment;
use crate::{ca65, frontend, lower_45gs02, lower_65816};

pub use crate::storage::DEFAULT_DIRECT_PAGE_BUDGET;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    /// Native-mode WDC 65816 (the x65-alike machine).
    #[default]
    W65816,
    /// MEGA65 45GS02, bare metal.
    Mega65,
}

impl Target {
    pub const ALL: [Target; 2] = [Target::W65816, Target::Mega65];

    pub fn name(self) -> &'static str {
        match self {
            Self::W65816 => "65816",
            Self::Mega65 => "mega65",
        }
    }

    /// The `--cpu` value ca65 needs for this target's output.
    pub fn ca65_cpu(self) -> &'static str {
        match self {
            Self::W65816 => "65816",
            Self::Mega65 => "45GS02",
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Target {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "65816" | "w65816" | "x65" => Ok(Self::W65816),
            "mega65" | "45gs02" | "m65" => Ok(Self::Mega65),
            other => Err(format!(
                "unknown target `{other}` (expected one of: 65816, mega65)"
            )),
        }
    }
}

#[derive(Debug)]
pub struct Compiled {
    pub asm: String,
    pub assignment: StorageAssignment,
}

pub fn compile_source(
    source: &str,
    target: Target,
    direct_page_budget: u16,
) -> Result<Compiled, String> {
    let module = frontend::compile_source(source).map_err(|error| error.to_string())?;
    compile_module(&module, target, direct_page_budget)
}

pub fn compile_module(
    module: &Module,
    target: Target,
    direct_page_budget: u16,
) -> Result<Compiled, String> {
    match target {
        Target::W65816 => {
            let (lowered, assignment) =
                lower_65816::lower_module_with_storage_budget(module, direct_page_budget)
                    .map_err(|error| error.to_string())?;
            Ok(Compiled {
                asm: ca65::emit_module(&lowered),
                assignment,
            })
        }
        Target::Mega65 => {
            let (lowered, assignment) =
                lower_45gs02::lower_module_with_storage_budget(module, direct_page_budget)
                    .map_err(|error| error.to_string())?;
            Ok(Compiled {
                asm: ca65::emit(&lowered),
                assignment,
            })
        }
    }
}
