use std::fmt;

pub use crate::asm::{
    BankedSymbol, DirectSymbol, FixedSymbol, ModuleEntry, ModuleImport, ModuleLibrary,
    ModuleMetadata, StaticData, StaticSymbol, VectorEntry,
};

pub type AsmModule = crate::asm::AsmModule<AsmInst>;
pub type AsmRoutine = crate::asm::AsmRoutine<AsmInst>;

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
    Ora(Operand),
    Eor(Operand),
    /// `asl a`
    AslA,
    /// `lsr a`
    LsrA,
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
    Bcc(String),
    Bcs(String),
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
    StackRelativeIndirectIndexedY(u8),
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

impl Symbol {
    fn rename(&mut self) {
        if let Symbol::Named(name) = self {
            *name = crate::asm::rename_tokens(name);
        }
    }
}

impl crate::asm::RenameSymbols for AsmInst {
    fn rename_symbols(&mut self) {
        use AsmInst::*;
        match self {
            Lda(operand) | Ldx(operand) | Ldy(operand) | Sta(operand) | Stx(operand)
            | Sty(operand) | Stz(operand) | Adc(operand) | Sbc(operand) | And(operand)
            | Ora(operand) | Eor(operand) | Inc(operand) | Dec(operand) | Cmp(operand)
            | Cpx(operand) => match operand {
                Operand::Immediate(_) | Operand::StackRelative(_)
                | Operand::StackRelativeIndirectIndexedY(_) => {}
                Operand::ImmediateSymbol(symbol)
                | Operand::Absolute(symbol)
                | Operand::AbsoluteIndexedX(symbol)
                | Operand::AbsoluteLong(symbol)
                | Operand::Direct(symbol)
                | Operand::DirectIndexedX(symbol)
                | Operand::DirectIndexedY(symbol)
                | Operand::IndirectDirect(symbol)
                | Operand::IndirectDirectIndexedY(symbol)
                | Operand::IndirectLongDirect(symbol)
                | Operand::IndirectLongDirectIndexedY(symbol) => symbol.rename(),
            },
            Pea(symbol) => symbol.rename(),
            Label(name) | Per(name) | Beq(name) | Bne(name) | Bcc(name) | Bcs(name)
            | Bra(name) | Brl(name) | Jsr(name) | Jsl(name) => {
                *name = crate::asm::rename_tokens(name)
            }
            _ => {}
        }
    }
}
