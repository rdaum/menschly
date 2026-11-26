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
    Dex,
    Iny,
    Dey,
    Inz,
    Dez,
    Clc,
    Sec,
    Adc,
    Sbc,
    And,
    Ora,
    Eor,
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
    /// 32-bit load of A, X, Y and Z (low byte first) from four bytes.
    Ldq,
    /// 32-bit store of A, X, Y and Z to four bytes.
    Stq,
    /// 32-bit add with carry of four bytes to A, X, Y and Z.
    Adcq,
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
    Bcc,
    Bcs,
    Bra,
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
            Self::Dex => "dex",
            Self::Iny => "iny",
            Self::Dey => "dey",
            Self::Inz => "inz",
            Self::Dez => "dez",
            Self::Clc => "clc",
            Self::Sec => "sec",
            Self::Adc => "adc",
            Self::Sbc => "sbc",
            Self::And => "and",
            Self::Ora => "ora",
            Self::Eor => "eor",
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
            Self::Ldq => "ldq",
            Self::Stq => "stq",
            Self::Adcq => "adcq",
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
            Self::Bcc => "bcc",
            Self::Bcs => "bcs",
            Self::Bra => "bra",
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

    /// Branches with a relative target: (short form, long form, inverse condition).
    pub fn branch_forms(self) -> Option<(Op, Op, Option<Op>)> {
        Some(match self {
            Self::Bne | Self::Lbne => (Self::Bne, Self::Lbne, Some(Self::Lbeq)),
            Self::Beq | Self::Lbeq => (Self::Beq, Self::Lbeq, Some(Self::Lbne)),
            Self::Bcc | Self::Lbcc => (Self::Bcc, Self::Lbcc, Some(Self::Lbcs)),
            Self::Bcs | Self::Lbcs => (Self::Bcs, Self::Lbcs, Some(Self::Lbcc)),
            Self::Bra | Self::Lbra => (Self::Bra, Self::Lbra, None),
            _ => return None,
        })
    }

    pub fn is_unconditional_branch(self) -> bool {
        matches!(self, Self::Bra | Self::Lbra)
    }

    /// Instructions after which execution never falls through.
    pub fn ends_flow(self) -> bool {
        matches!(
            self,
            Self::Bra | Self::Lbra | Self::Rts | Self::Rtn | Self::Rti
        )
    }
}

impl Inst {
    /// Encoded size in bytes, or `None` for raw assembly text.
    pub fn size(&self) -> Option<u16> {
        let Inst::Op(op, operand) = self else {
            return match self {
                Inst::Label(_) => Some(0),
                _ => None,
            };
        };
        // The 32-bit forms are the 8-bit ones behind a NEG NEG prefix.
        let prefix = if matches!(op, Op::Ldq | Op::Stq | Op::Adcq) {
            2
        } else {
            0
        };
        Some(prefix + match operand {
            Operand::Implied => 1,
            Operand::Immediate(_) | Operand::Base(_) => 2,
            Operand::IndirectZ(_) | Operand::IndirectY(_) => 2,
            Operand::Absolute(_) | Operand::AbsoluteX(_) | Operand::Indirect(_) => 3,
            // `[bp],z` is `(bp),z` behind a NOP prefix.
            Operand::FlatZ(_) => 3,
            Operand::Label(_) => match op {
                Op::Bne | Op::Beq | Op::Bcc | Op::Bcs | Op::Bra => 2,
                _ => 3,
            },
        })
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
