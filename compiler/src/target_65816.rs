use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmModule {
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
    pub routines: Vec<AsmRoutine>,
}

impl AsmModule {
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

    pub fn with_routine(mut self, routine: AsmRoutine) -> Self {
        self.routines.push(routine);
        self
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
pub struct AsmRoutine {
    pub name: String,
    pub export: bool,
    pub insts: Vec<AsmInst>,
}

impl AsmRoutine {
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

    pub fn with_inst(mut self, inst: AsmInst) -> Self {
        self.insts.push(inst);
        self
    }

    pub fn with_insts(mut self, insts: Vec<AsmInst>) -> Self {
        self.insts = insts;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsmInst {
    Label(String),
    Mode(Mode),
    Lda(Operand),
    Ldx(Operand),
    Ldy(Operand),
    Sta(Operand),
    Stx(Operand),
    Sty(Operand),
    Stz(Operand),
    Tax,
    Tay,
    Txa,
    Tya,
    Tsc,
    Tcs,
    Inx,
    Clc,
    Sec,
    Adc(Operand),
    Sbc(Operand),
    And(Operand),
    Xba,
    Pha,
    Phx,
    Phy,
    Php,
    Phb,
    Phk,
    Pea(Symbol),
    Per(String),
    Pla,
    Plx,
    Ply,
    Plp,
    Plb,
    Inc(Operand),
    Dec(Operand),
    Cmp(Operand),
    Cpx(Operand),
    Beq(String),
    Bne(String),
    Bra(String),
    Brl(String),
    Jsr(String),
    Jsl(String),
    Rts,
    Rtl,
    Rti,
    Nop,
    Stp,
    Raw(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Acc8Index16,
    Acc16Index16,
    Acc8Index8,
    Acc16Index8,
}

impl Mode {
    pub fn directives(self) -> &'static [&'static str] {
        match self {
            Self::Acc8Index16 => &["sep #$20", ".a8", ".i16"],
            Self::Acc16Index16 => &["rep #$30", ".a16", ".i16"],
            Self::Acc8Index8 => &["sep #$30", ".a8", ".i8"],
            Self::Acc16Index8 => &["rep #$20", "sep #$10", ".a16", ".i8"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Immediate(Immediate),
    ImmediateSymbol(Symbol),
    Absolute(Symbol),
    AbsoluteIndexedX(Symbol),
    AbsoluteLong(Symbol),
    Direct(Symbol),
    DirectIndexedX(Symbol),
    DirectIndexedY(Symbol),
    StackRelative(u8),
    IndirectDirect(Symbol),
    IndirectDirectIndexedY(Symbol),
    IndirectLongDirect(Symbol),
    IndirectLongDirectIndexedY(Symbol),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Immediate {
    Byte(u8),
    Word(u16),
    Long(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Symbol {
    Named(String),
    Address(u32),
}

impl From<&str> for Symbol {
    fn from(value: &str) -> Self {
        Self::Named(value.to_owned())
    }
}

impl From<String> for Symbol {
    fn from(value: String) -> Self {
        Self::Named(value)
    }
}

impl From<u32> for Symbol {
    fn from(value: u32) -> Self {
        Self::Address(value)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => f.write_str(name),
            Self::Address(address) => write!(f, "${address:06x}"),
        }
    }
}
