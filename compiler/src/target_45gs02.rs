//! 45GS02 (MEGA65) instruction representation.
//!
//! The 45GS02 is a 65CE02 derivative: 8-bit A/X/Y/Z, a relocatable base page
//! (the 6502 zero page, moved with `TAB`), 16-bit relative branches, word
//! increment/decrement on base-page operands, and 32-bit flat addressing via
//! `[bp],z`.
//!
//! Code generated for this target relies on one register invariant: **Z is
//! zero** outside of the short sequences that index through `(bp),z` or
//! `[bp],z`. That makes `(bp),z` behave like the 65C02's `(bp)` and lets the
//! 65CE02 `STZ` (which stores Z, not a literal zero) clear memory.

use crate::ca65::Ca65Inst;

pub type AsmModule = crate::asm::AsmModule<Inst>;
pub type AsmRoutine = crate::asm::AsmRoutine<Inst>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inst {
    Label(String),
    Op(Op, Operand),
    Raw(String),
}

impl Inst {
    pub fn op(op: Op, operand: Operand) -> Self {
        Self::Op(op, operand)
    }

    pub fn implied(op: Op) -> Self {
        Self::Op(op, Operand::Implied)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Lda,
    Ldx,
    Ldy,
    Ldz,
    Sta,
    Stx,
    Sty,
    /// Stores the Z register (zero under the codegen invariant).
    Stz,
    Tax,
    Tay,
    Txa,
    Tya,
    Tsx,
    Tsy,
    Inx,
    Clc,
    Sec,
    Adc,
    Sbc,
    And,
    Ora,
    Cmp,
    Cpx,
    Inc,
    Dec,
    Asl,
    Rol,
    Lsr,
    Ror,
    /// 16-bit increment of a base-page word.
    Inw,
    /// 16-bit decrement of a base-page word.
    Dew,
    Pha,
    Phx,
    Phy,
    Phz,
    Pla,
    Plx,
    Ply,
    Plz,
    Bne,
    Beq,
    Lbra,
    Lbeq,
    Lbne,
    Lbcc,
    Lbcs,
    Jsr,
    Rts,
    /// 65CE02 `RTN #n`: return, then drop `n` argument bytes.
    Rtn,
    Rti,
}

impl Op {
    fn mnemonic(self) -> &'static str {
        match self {
            Self::Lda => "lda",
            Self::Ldx => "ldx",
            Self::Ldy => "ldy",
            Self::Ldz => "ldz",
            Self::Sta => "sta",
            Self::Stx => "stx",
            Self::Sty => "sty",
            Self::Stz => "stz",
            Self::Tax => "tax",
            Self::Tay => "tay",
            Self::Txa => "txa",
            Self::Tya => "tya",
            Self::Tsx => "tsx",
            Self::Tsy => "tsy",
            Self::Inx => "inx",
            Self::Clc => "clc",
            Self::Sec => "sec",
            Self::Adc => "adc",
            Self::Sbc => "sbc",
            Self::And => "and",
            Self::Ora => "ora",
            Self::Cmp => "cmp",
            Self::Cpx => "cpx",
            Self::Inc => "inc",
            Self::Asl => "asl",
            Self::Rol => "rol",
            Self::Lsr => "lsr",
            Self::Ror => "ror",
            Self::Dec => "dec",
            Self::Inw => "inw",
            Self::Dew => "dew",
            Self::Pha => "pha",
            Self::Phx => "phx",
            Self::Phy => "phy",
            Self::Phz => "phz",
            Self::Pla => "pla",
            Self::Plx => "plx",
            Self::Ply => "ply",
            Self::Plz => "plz",
            Self::Bne => "bne",
            Self::Beq => "beq",
            Self::Lbra => "lbra",
            Self::Lbeq => "lbeq",
            Self::Lbne => "lbne",
            Self::Lbcc => "lbcc",
            Self::Lbcs => "lbcs",
            Self::Jsr => "jsr",
            Self::Rts => "rts",
            Self::Rtn => "rtn",
            Self::Rti => "rti",
        }
    }
}

/// Operands. Symbol text may already carry a `+offset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Implied,
    /// Immediate expression, printed after `#` (e.g. `$12`, `<label`).
    Immediate(String),
    Absolute(String),
    AbsoluteX(String),
    /// Base-page operand.
    Base(String),
    /// `(bp),z`
    IndirectZ(String),
    /// `(bp),y`
    IndirectY(String),
    /// `[bp],z` — 32-bit flat pointer.
    FlatZ(String),
    /// Branch/jump target.
    Label(String),
    /// `(addr)` — indirect jump/call through a vector.
    Indirect(String),
}

impl Operand {
    pub fn imm_byte(value: u8) -> Self {
        Self::Immediate(format!("${value:02x}"))
    }

    pub fn is_zero_immediate(&self) -> bool {
        matches!(self, Self::Immediate(text) if text == "$00")
    }
}

impl Ca65Inst for Inst {
    const CPU_DIRECTIVE: &'static str = ".setcpu \"45GS02\"";

    fn label(&self) -> Option<&str> {
        match self {
            Inst::Label(label) => Some(label),
            _ => None,
        }
    }

    fn emit(&self) -> String {
        match self {
            Inst::Label(_) => unreachable!("labels are emitted by the routine emitter"),
            Inst::Raw(line) => line.clone(),
            Inst::Op(op, operand) => {
                let mnemonic = op.mnemonic();
                match operand {
                    Operand::Implied => mnemonic.to_owned(),
                    Operand::Immediate(value) => format!("{mnemonic} #{value}"),
                    Operand::Absolute(symbol) => format!("{mnemonic} a:{symbol}"),
                    Operand::AbsoluteX(symbol) => format!("{mnemonic} a:{symbol},x"),
                    Operand::Base(symbol) => format!("{mnemonic} z:{symbol}"),
                    Operand::IndirectZ(symbol) => format!("{mnemonic} ({symbol}),z"),
                    Operand::IndirectY(symbol) => format!("{mnemonic} ({symbol}),y"),
                    Operand::FlatZ(symbol) => format!("{mnemonic} [{symbol}],z"),
                    Operand::Label(label) => format!("{mnemonic} {label}"),
                    Operand::Indirect(address) => format!("{mnemonic} ({address})"),
                }
            }
        }
    }
}
