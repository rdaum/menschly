//! Peephole and dataflow cleanup for lowered 45GS02 routines.
//!
//! The lowering emits each IR instruction byte by byte in isolation, which
//! leaves a lot of mechanical waste: values stored to the base-page
//! accumulator and immediately reloaded, frame offsets reloaded into `Y`,
//! `CMP #0` after a load that already set the flags, and 3-byte long
//! branches where a 2-byte short one reaches. This pass removes that waste
//! with four steps, repeated until nothing changes:
//!
//! 1. **Jump cleanup** — unreachable code, branches to the next instruction,
//!    a conditional branch over an unconditional one, and jumps to jumps.
//! 2. **Dead-code elimination** — a backward liveness analysis over the
//!    routine's control-flow graph tracks A/X/Y/Z, the N/Z and C flags, and
//!    each byte of the base-page runtime registers. Side-effect-free
//!    instructions whose results are all dead are deleted.
//! 3. **Redundant loads and stores** — a forward pass tracks which known
//!    values each register holds (immediates, and the contents of memory the
//!    compiler owns). Loading a value already present, or storing a value
//!    memory already holds, is deleted when the flags it would set are dead;
//!    `CMP #0` is deleted when N/Z already reflect A and carry is dead.
//! 4. **Branch relaxation** (once, at the end) — branches become the short
//!    form wherever the target is in range.
//!
//! Only memory the compiler itself allocates is ever treated as ordinary RAM:
//! the runtime registers, `__`-prefixed storage symbols, and frame bytes.
//! User globals may be volatile I/O registers, so accesses to them are kept
//! exactly as written. Routines containing inline assembly are not touched.

use std::collections::{HashMap, HashSet};

use crate::lower_45gs02::{
    ACC_SYMBOL, ARG_SYMBOL, FARP_SYMBOL, FP_SYMBOL, LEAVE_ROUTINE, MATH_ROUTINES, PTR_SYMBOL,
    TMP_SYMBOL,
};
use crate::target_45gs02::{Inst, Op, Operand};

/// A set of tracked machine locations, one bit each.
type Locs = u32;

const A: Locs = 1 << 0;
const X: Locs = 1 << 1;
const Y: Locs = 1 << 2;
const Z: Locs = 1 << 3;
/// The N and Z flags (always written together here).
const NZ: Locs = 1 << 4;
const C: Locs = 1 << 5;
const ACC: Locs = 0xf << 6;
const PTR: Locs = 0x3 << 10;
const FARP: Locs = 0xf << 12;
const FP: Locs = 0x3 << 16;
const ARG: Locs = 0xf << 18;
const TMP: Locs = 0xff << 22;
/// The top byte of the accumulator, which carries a 32-bit result's byte 3.
const ACC3: Locs = 1 << 9;
const ALL: Locs = (1 << 30) - 1;

/// Runtime base-page registers: (symbol, first bit, bytes).
const RUNTIME: [(&str, u32, u32); 6] = [
    (ACC_SYMBOL, 6, 4),
    (PTR_SYMBOL, 10, 2),
    (FARP_SYMBOL, 12, 4),
    (FP_SYMBOL, 16, 2),
    (ARG_SYMBOL, 18, 4),
    (TMP_SYMBOL, 22, 8),
];

/// Optimizes one routine. `z_zero_on_entry` is false for interrupt handlers,
/// which may interrupt a `(bp),z` sequence with Z non-zero.
pub fn optimize_routine(insts: Vec<Inst>, z_zero_on_entry: bool) -> Vec<Inst> {
    optimize_routine_with(insts, z_zero_on_entry, &HashSet::new(), false)
}

/// Like [`optimize_routine`]; `flag_routines` are routines known to return
/// with N/Z describing A (see `lower_45gs02`), so a `CMP #0` of their
/// result can go.
pub fn optimize_routine_with(
    insts: Vec<Inst>,
    z_zero_on_entry: bool,
    flag_routines: &HashSet<String>,
    returns_carry: bool,
) -> Vec<Inst> {
    if insts.iter().any(|inst| matches!(inst, Inst::Raw(_))) {
        return insts;
    }
    let mut insts = insts;
    loop {
        let before = insts.len();
        let mut changed = clean_jumps(&mut insts);
        changed |= eliminate_dead_code(&mut insts, returns_carry);
        changed |= eliminate_redundant(&mut insts, z_zero_on_entry, flag_routines, returns_carry);
        changed |= carry_increments(&mut insts, returns_carry);
        if !changed && insts.len() == before {
            break;
        }
    }
    relax_branches(&mut insts);
    insts
}

// ---- operands and effects ---------------------------------------------------

fn split_offset(symbol: &str) -> (&str, Option<u32>) {
    match symbol.split_once('+') {
        Some((name, offset)) => (name, offset.parse().ok()),
        None => (symbol, Some(0)),
    }
}

/// The bit for one byte of a runtime register, if `symbol` names one.
fn runtime_byte(symbol: &str) -> Option<Locs> {
    let (name, offset) = split_offset(symbol);
    let offset = offset?;
    RUNTIME
        .iter()
        .find(|(runtime, _, _)| *runtime == name)
        .and_then(|(_, bit, bytes)| (offset < *bytes).then(|| 1 << (bit + offset)))
}

/// The bits for `bytes` consecutive bytes starting at base-page `symbol`.
fn runtime_bytes(symbol: &str, bytes: u32) -> Locs {
    let (name, offset) = split_offset(symbol);
    let Some(offset) = offset else { return 0 };
    (0..bytes)
        .filter_map(|i| runtime_byte(&format!("{name}+{}", offset + i)))
        .fold(0, |set, bit| set | bit)
}

fn is_runtime_symbol(symbol: &str) -> bool {
    RUNTIME
        .iter()
        .any(|(runtime, _, _)| split_offset(symbol).0 == *runtime)
}

/// Storage the compiler allocated itself (never a volatile hardware register).
fn is_compiler_symbol(symbol: &str) -> bool {
    symbol.starts_with("__")
}

/// Index/pointer registers an operand's addressing mode reads.
fn address_uses(operand: &Operand) -> Locs {
    match operand {
        Operand::AbsoluteX(_) => X,
        Operand::IndirectY(pointer) => Y | runtime_bytes(pointer, 2),
        Operand::IndirectZ(pointer) => Z | runtime_bytes(pointer, 2),
        Operand::FlatZ(pointer) => Z | runtime_bytes(pointer, 4),
        _ => 0,
    }
}

/// The tracked location a memory operand names directly, if any.
fn direct_loc(operand: &Operand) -> Locs {
    match operand {
        Operand::Base(symbol) => runtime_byte(symbol).unwrap_or(0),
        _ => 0,
    }
}

/// Whether reading the operand can have no side effect.
fn pure_read(operand: &Operand) -> bool {
    match operand {
        Operand::Implied | Operand::Immediate(_) => true,
        Operand::Base(symbol) | Operand::Absolute(symbol) => is_compiler_symbol(symbol),
        Operand::IndirectY(pointer) => pointer == FP_SYMBOL,
        _ => false,
    }
}

fn register_bit(op: Op) -> Locs {
    match op {
        Op::Lda | Op::Sta | Op::Pha | Op::Pla => A,
        Op::Ldx | Op::Stx | Op::Phx | Op::Plx => X,
        Op::Ldy | Op::Sty | Op::Phy | Op::Ply => Y,
        Op::Ldz | Op::Stz | Op::Phz | Op::Plz => Z,
        _ => 0,
    }
}

struct Effect {
    uses: Locs,
    defs: Locs,
    /// The instruction does nothing besides writing `defs`.
    removable: bool,
}

fn effect(inst: &Inst) -> Effect {
    let none = Effect {
        uses: 0,
        defs: 0,
        removable: false,
    };
    let Inst::Op(op, operand) = inst else {
        return none;
    };
    let op = *op;
    let read = address_uses(operand) | direct_loc(operand);
    let (uses, defs, removable) = match op {
        Op::Lda | Op::Ldx | Op::Ldy | Op::Ldz => (read, register_bit(op) | NZ, pure_read(operand)),
        Op::Sta | Op::Stx | Op::Sty | Op::Stz => {
            let target = direct_loc(operand);
            (
                register_bit(op) | address_uses(operand),
                target,
                target != 0,
            )
        }
        Op::Tax => (A, X | NZ, true),
        Op::Tay => (A, Y | NZ, true),
        Op::Txa => (X, A | NZ, true),
        Op::Tya => (Y, A | NZ, true),
        Op::Tsx => (0, X | NZ, true),
        Op::Tsy => (0, Y | NZ, true),
        Op::Inx | Op::Dex => (X, X | NZ, true),
        Op::Iny | Op::Dey => (Y, Y | NZ, true),
        Op::Inz | Op::Dez => (Z, Z | NZ, true),
        Op::Clc | Op::Sec => (0, C, true),
        Op::Adc | Op::Sbc => (A | C | read, A | NZ | C, pure_read(operand)),
        Op::And | Op::Ora | Op::Eor => (A | read, A | NZ, pure_read(operand)),
        Op::Cmp => (A | read, NZ | C, pure_read(operand)),
        Op::Cpx => (X | read, NZ | C, pure_read(operand)),
        Op::Inc | Op::Dec | Op::Asl | Op::Lsr | Op::Rol | Op::Ror => {
            let carry_in = if matches!(op, Op::Rol | Op::Ror) {
                C
            } else {
                0
            };
            let carry_out = if matches!(op, Op::Inc | Op::Dec) {
                0
            } else {
                C
            };
            if matches!(operand, Operand::Implied) {
                (A | carry_in, A | NZ | carry_out, true)
            } else {
                let target = direct_loc(operand);
                (read | carry_in, target | NZ | carry_out, target != 0)
            }
        }
        Op::Inw | Op::Dew => {
            let Operand::Base(symbol) = operand else {
                return none;
            };
            let target = runtime_bytes(symbol, 2);
            let whole = target.count_ones() == 2;
            (target, target | NZ, whole)
        }
        Op::Pha | Op::Phx | Op::Phy | Op::Phz => (register_bit(op), 0, false),
        Op::Pla | Op::Plx | Op::Ply | Op::Plz => (0, register_bit(op) | NZ, false),
        Op::Bne | Op::Beq | Op::Lbne | Op::Lbeq => (NZ, 0, false),
        Op::Bcc | Op::Bcs | Op::Lbcc | Op::Lbcs => (C, 0, false),
        Op::Bra | Op::Lbra => (0, 0, false),
        // Callees take their arguments on the stack or in their own
        // parameter storage, never in the base-page scratch registers (except
        // the math helpers, which take theirs in ACC and ARG); they return
        // results in A/X/Y (and ACC+3), keep Z = 0 and the frame pointer, and
        // clobber the scratch registers. An indirect call reads its vector.
        Op::Jsr => {
            let arguments = match operand {
                Operand::Label(name) if MATH_ROUTINES.contains(&name.as_str()) => ACC | ARG,
                Operand::Indirect(vector) => runtime_bytes(vector, 2),
                _ => 0,
            };
            // The math routines only touch their own operands and the flat
            // pointer, so temp registers survive them.
            let clobbers = match operand {
                Operand::Label(name) if MATH_ROUTINES.contains(&name.as_str()) => {
                    A | X | Y | NZ | C | ACC | FARP | ARG
                }
                // The frame release keeps the carry (a carry result).
                Operand::Label(name) if name == LEAVE_ROUTINE => {
                    A | X | Y | NZ | ACC | PTR | FARP | ARG | TMP
                }
                _ => A | X | Y | NZ | C | ACC | PTR | FARP | ARG | TMP,
            };
            (A | X | Y | Z | FP | arguments, clobbers, false)
        }
        // Results are in A/X/Y, and a 32-bit result's top byte in ACC+3;
        // N/Z describe A for 1- and 2-byte results.
        Op::Rts | Op::Rtn => (A | X | Y | Z | FP | ACC3 | NZ, 0, false),
        Op::Rti => (ALL, 0, false),
    };
    Effect {
        uses,
        defs,
        removable,
    }
}

fn branch_target(inst: &Inst) -> Option<&str> {
    match inst {
        Inst::Op(op, Operand::Label(label)) if op.branch_forms().is_some() => Some(label),
        _ => None,
    }
}

fn label_positions(insts: &[Inst]) -> HashMap<String, usize> {
    insts
        .iter()
        .enumerate()
        .filter_map(|(i, inst)| match inst {
            Inst::Label(label) => Some((label.clone(), i)),
            _ => None,
        })
        .collect()
}

// ---- liveness ---------------------------------------------------------------

/// Locations live immediately after each instruction.
/// `returns_carry`: the routine returns a result in the carry flag.
fn live_after(insts: &[Inst], returns_carry: bool) -> Vec<Locs> {
    let labels = label_positions(insts);
    let mut effects: Vec<Effect> = insts.iter().map(effect).collect();
    if returns_carry {
        for (inst, effect) in insts.iter().zip(&mut effects) {
            if matches!(inst, Inst::Op(Op::Rts | Op::Rtn, _)) {
                effect.uses |= C;
            }
        }
    }
    let n = insts.len();
    // Falling off the end of the list is treated as leaving everything live.
    let mut live_in = vec![0 as Locs; n + 1];
    live_in[n] = ALL;
    let mut live_out = vec![0 as Locs; n];
    loop {
        let mut changed = false;
        for i in (0..n).rev() {
            let falls_through = match &insts[i] {
                Inst::Op(op, _) => !op.ends_flow(),
                _ => true,
            };
            let mut out = if falls_through { live_in[i + 1] } else { 0 };
            if let Some(target) = branch_target(&insts[i]) {
                out |= labels.get(target).map_or(ALL, |&at| live_in[at]);
            }
            let input = effects[i].uses | (out & !effects[i].defs);
            if out != live_out[i] || input != live_in[i] {
                live_out[i] = out;
                live_in[i] = input;
                changed = true;
            }
        }
        if !changed {
            return live_out;
        }
    }
}

fn eliminate_dead_code(insts: &mut Vec<Inst>, returns_carry: bool) -> bool {
    let live = live_after(insts, returns_carry);
    let before = insts.len();
    let mut i = 0;
    insts.retain(|inst| {
        let effect = effect(inst);
        let dead = effect.removable && effect.defs != 0 && effect.defs & live[i] == 0;
        i += 1;
        !dead
    });
    insts.len() != before
}

// ---- known register values --------------------------------------------------

/// What each register is known to equal: immediates (`#…`), memory the
/// compiler owns (`z:…`, `a:…`, `fp:…` for frame bytes), and otherwise
/// unknown values numbered as they appear (`v:…`), so that copies between
/// registers can be recognized.
#[derive(Default, Clone)]
struct Known {
    regs: [Vec<String>; 4],
    /// Whether N/Z currently describe the value in A.
    nz_from_a: bool,
    next_value: usize,
    /// Memory known to hold a copy of other memory: (copy, original), by
    /// key, from a store of a register that held the original.
    copies: Vec<(String, String)>,
}

const REG_A: usize = 0;
const REG_X: usize = 1;
const REG_Y: usize = 2;
const REG_Z: usize = 3;

fn reg_index(op: Op) -> Option<usize> {
    match register_bit(op) {
        A => Some(REG_A),
        X => Some(REG_X),
        Y => Some(REG_Y),
        Z => Some(REG_Z),
        _ => None,
    }
}

impl Known {
    fn entry() -> Self {
        let mut known = Self::default();
        // Z is zero on entry to every routine (the codegen invariant).
        known.regs[REG_Z] = vec!["#$00".to_owned()];
        known
    }

    fn holds(&self, reg: usize, key: &str) -> bool {
        self.regs[reg].iter().any(|held| held == key)
    }

    fn set(&mut self, reg: usize, key: Option<String>) {
        match key {
            Some(key) => self.regs[reg] = vec![key],
            None => self.forget(reg),
        }
    }

    /// The register now holds a new, unknown value.
    fn forget(&mut self, reg: usize) {
        self.next_value += 1;
        self.regs[reg] = vec![format!("v:{}", self.next_value)];
    }

    /// The register holding the value at `key`, other than `except`.
    fn holder(&self, key: &str, except: usize) -> Option<usize> {
        (0..4).find(|&reg| reg != except && self.holds(reg, key))
    }

    /// The immediate a key is known to equal (a register holds both).
    fn constant(&self, key: &str) -> Option<String> {
        self.regs.iter().find_map(|keys| {
            keys.iter().any(|held| held == key).then(|| {
                keys.iter()
                    .find_map(|held| held.strip_prefix('#').map(str::to_owned))
            })?
        })
    }

    /// Whether two registers are known to hold the same value.
    fn same(&self, a: usize, b: usize) -> bool {
        self.regs[a].iter().any(|key| self.holds(b, key))
    }

    fn retain_keys(&mut self, keep: impl Fn(&str) -> bool) {
        for reg in &mut self.regs {
            reg.retain(|key| keep(key));
        }
        self.copies
            .retain(|(copy, original)| keep(copy) && keep(original));
    }

    /// Whether memory `key` holds a copy of a value register `reg` holds.
    fn holds_copy_of(&self, key: &str, reg: usize) -> bool {
        self.copies
            .iter()
            .any(|(copy, original)| copy == key && self.holds(reg, original))
    }

    /// The frame byte `(fp),y` names, when Y is a known immediate.
    fn frame_key(&self) -> Option<String> {
        self.regs[REG_Y]
            .iter()
            .find_map(|key| key.strip_prefix('#'))
            .map(|offset| format!("fp:{offset}"))
    }

    /// A key for the value an operand reads, if it is a known kind.
    fn key(&self, operand: &Operand) -> Option<String> {
        match operand {
            Operand::Immediate(text) => Some(format!("#{text}")),
            Operand::Base(symbol) if is_compiler_symbol(symbol) => Some(format!("z:{symbol}")),
            Operand::Absolute(symbol) if is_compiler_symbol(symbol) => Some(format!("a:{symbol}")),
            Operand::IndirectY(pointer) if pointer == FP_SYMBOL => self.frame_key(),
            _ => None,
        }
    }

    /// Forgets whatever memory a write to `operand` (of `bytes` bytes) may change.
    fn clobber(&mut self, operand: &Operand, bytes: u32) {
        match operand {
            Operand::Base(symbol) | Operand::Absolute(symbol) if is_compiler_symbol(symbol) => {
                let prefix = if matches!(operand, Operand::Base(_)) {
                    "z:"
                } else {
                    "a:"
                };
                let (name, offset) = split_offset(symbol);
                let Some(offset) = offset else {
                    self.clobber_all_memory();
                    return;
                };
                for i in 0..bytes {
                    let byte = if offset + i == 0 {
                        name.to_owned()
                    } else {
                        format!("{name}+{}", offset + i)
                    };
                    let key = format!("{prefix}{byte}");
                    self.retain_keys(|held| held != key);
                }
                if name == FP_SYMBOL {
                    self.retain_keys(|held| !held.starts_with("fp:"));
                }
            }
            // A different, user-named symbol cannot alias compiler storage.
            Operand::Base(symbol) | Operand::Absolute(symbol) if !symbol.starts_with('$') => {}
            Operand::IndirectY(pointer) if pointer == FP_SYMBOL => match self.frame_key() {
                Some(key) => self.retain_keys(|held| held != key),
                None => self.retain_keys(|held| !held.starts_with("fp:")),
            },
            // Through a pointer: Mensch takes no address of compiler-owned
            // storage (locals, parameters, temporaries), so only the stack
            // frame may be reached.
            Operand::IndirectY(_) | Operand::IndirectZ(_) | Operand::FlatZ(_) => {
                self.retain_keys(|held| !held.starts_with("fp:"))
            }
            _ => self.clobber_all_memory(),
        }
    }

    /// Forgets every memory value except the runtime registers, which no
    /// pointer can address.
    fn clobber_all_memory(&mut self) {
        self.retain_keys(|key| {
            key.starts_with('#')
                || key.starts_with("v:")
                || key.strip_prefix("z:").is_some_and(is_runtime_symbol)
        });
    }
}

fn eliminate_redundant(
    insts: &mut Vec<Inst>,
    z_zero_on_entry: bool,
    flag_routines: &HashSet<String>,
    returns_carry: bool,
) -> bool {
    let live = live_after(insts, returns_carry);
    let referenced: HashSet<String> = insts
        .iter()
        .filter_map(branch_target)
        .map(str::to_owned)
        .collect();
    let mut known = if z_zero_on_entry {
        Known::entry()
    } else {
        Known::default()
    };
    let mut delete = vec![false; insts.len()];
    let mut rewrite = Vec::new();
    // Z is zero at every branch target (the codegen invariant), once an
    // interrupt handler's prologue has established it.
    let mut z_established = z_zero_on_entry;
    for (i, inst) in insts.iter().enumerate() {
        let (op, operand) = match inst {
            Inst::Label(label) => {
                if referenced.contains(label) {
                    let next_value = known.next_value;
                    known = if z_established {
                        Known::entry()
                    } else {
                        Known::default()
                    };
                    known.next_value = next_value;
                }
                continue;
            }
            Inst::Raw(_) => unreachable!("routines with raw assembly are not optimized"),
            Inst::Op(op, operand) => (*op, operand),
        };
        let flags_dead = live[i] & NZ == 0;
        let carry_dead = live[i] & C == 0;
        if op == Op::Ldz && operand.is_zero_immediate() {
            z_established = true;
        }
        match op {
            Op::Lda | Op::Ldx | Op::Ldy | Op::Ldz => {
                let reg = reg_index(op).expect("load register");
                let key = known.key(operand);
                // Reloading A also leaves N/Z alone when they describe A.
                let flags_kept = flags_dead || (op == Op::Lda && known.nz_from_a);
                if flags_kept && key.as_ref().is_some_and(|key| known.holds(reg, key)) {
                    // The register and the flags are exactly as before.
                    delete[i] = true;
                    continue;
                }
                // Another register holds the value: transfer it (1 byte, the
                // same N/Z).
                if let Some(key) = key.as_ref().filter(|key| !key.starts_with('#')) {
                    let transfer = match (reg, known.holder(key, reg)) {
                        (REG_X, Some(REG_A)) => Some(Op::Tax),
                        (REG_Y, Some(REG_A)) => Some(Op::Tay),
                        (REG_A, Some(REG_X)) => Some(Op::Txa),
                        (REG_A, Some(REG_Y)) => Some(Op::Tya),
                        _ => None,
                    };
                    if let Some(transfer) = transfer {
                        let from = if matches!(transfer, Op::Tax | Op::Tay) {
                            REG_A
                        } else {
                            known.holder(key, reg).expect("holder")
                        };
                        rewrite.push((i, Inst::implied(transfer)));
                        known.regs[reg] = known.regs[from].clone();
                        known.nz_from_a = true;
                        continue;
                    }
                    // Memory known to hold a constant reads as the constant.
                    if let Some(constant) = known.constant(key) {
                        rewrite.push((i, Inst::op(op, Operand::Immediate(constant.clone()))));
                        known.set(reg, Some(format!("#{constant}")));
                        known.regs[reg].push(key.clone());
                        known.nz_from_a = op == Op::Lda;
                        continue;
                    }
                }
                // A known neighbouring immediate is one step away; INY & co.
                // set N/Z exactly as the load would.
                if let (Some(step), Operand::Immediate(text)) = (step_ops(op), operand) {
                    if let Some(new) = parse_byte(text) {
                        let old = known.regs[reg]
                            .iter()
                            .find_map(|key| key.strip_prefix('#').and_then(parse_byte));
                        let replacement = match old {
                            Some(old) if old.wrapping_add(1) == new => Some(step.0),
                            Some(old) if old.wrapping_sub(1) == new => Some(step.1),
                            _ => None,
                        };
                        if let Some(replacement) = replacement {
                            rewrite.push((i, Inst::implied(replacement)));
                        }
                    }
                }
                known.set(reg, key);
                known.nz_from_a = op == Op::Lda;
            }
            Op::Sta | Op::Stx | Op::Sty | Op::Stz => {
                let reg = reg_index(op).expect("store register");
                let key = known.key(operand);
                // The memory already holds the register's value, or a copy
                // of the memory the register was loaded from.
                if key
                    .as_ref()
                    .is_some_and(|key| known.holds(reg, key) || known.holds_copy_of(key, reg))
                {
                    delete[i] = true;
                    continue;
                }
                known.clobber(operand, 1);
                if let Some(key) = key {
                    let originals: Vec<String> = known.regs[reg]
                        .iter()
                        .filter(|held| {
                            held.starts_with("a:")
                                || held.starts_with("z:")
                                || held.starts_with("fp:")
                        })
                        .cloned()
                        .collect();
                    for original in originals {
                        known.copies.push((key.clone(), original));
                    }
                    known.regs[reg].push(key);
                }
            }
            Op::Tax | Op::Tay | Op::Txa | Op::Tya => {
                let index = if matches!(op, Op::Tax | Op::Txa) {
                    REG_X
                } else {
                    REG_Y
                };
                let (from, to) = if matches!(op, Op::Tax | Op::Tay) {
                    (REG_A, index)
                } else {
                    (index, REG_A)
                };
                // Copying a register into one that already holds its value
                // changes nothing (N/Z too, when they describe A).
                if known.same(from, to) && (flags_dead || known.nz_from_a) {
                    delete[i] = true;
                    continue;
                }
                known.regs[to] = known.regs[from].clone();
                known.nz_from_a = true;
            }
            Op::Cmp if operand.is_zero_immediate() && known.nz_from_a && carry_dead => {
                delete[i] = true;
            }
            Op::And | Op::Ora
                if matches!(operand, Operand::Immediate(text)
                    if (op == Op::And && text == "$ff") || (op == Op::Ora && text == "$00"))
                    && (known.nz_from_a || flags_dead) =>
            {
                delete[i] = true;
            }
            Op::Adc | Op::Sbc | Op::And | Op::Ora | Op::Eor | Op::Pla => {
                if op != Op::Pla {
                    if let Some(constant) = known
                        .key(operand)
                        .filter(|key| !key.starts_with('#'))
                        .and_then(|key| known.constant(&key))
                    {
                        rewrite.push((i, Inst::op(op, Operand::Immediate(constant))));
                    }
                }
                known.forget(REG_A);
                known.nz_from_a = true;
            }
            Op::Inc | Op::Dec | Op::Asl | Op::Lsr | Op::Rol | Op::Ror | Op::Inw | Op::Dew => {
                if matches!(operand, Operand::Implied) {
                    known.forget(REG_A);
                    known.nz_from_a = true;
                } else {
                    known.clobber(
                        operand,
                        if matches!(op, Op::Inw | Op::Dew) {
                            2
                        } else {
                            1
                        },
                    );
                    known.nz_from_a = false;
                }
            }
            Op::Tsx | Op::Inx | Op::Dex | Op::Plx => {
                known.forget(REG_X);
                known.nz_from_a = false;
            }
            Op::Tsy | Op::Iny | Op::Dey | Op::Ply => {
                known.forget(REG_Y);
                known.nz_from_a = false;
            }
            Op::Plz | Op::Inz | Op::Dez => {
                known.forget(REG_Z);
                known.nz_from_a = false;
            }
            Op::Cmp | Op::Cpx => known.nz_from_a = false,
            Op::Jsr => {
                let next_value = known.next_value;
                known = Known::entry();
                known.next_value = next_value;
                for reg in [REG_A, REG_X, REG_Y] {
                    known.forget(reg);
                }
                known.nz_from_a =
                    matches!(operand, Operand::Label(name) if flag_routines.contains(name));
            }
            Op::Clc | Op::Sec | Op::Pha | Op::Phx | Op::Phy | Op::Phz => {}
            Op::Bne | Op::Beq | Op::Bcc | Op::Bcs | Op::Lbne | Op::Lbeq | Op::Lbcc | Op::Lbcs => {}
            Op::Bra | Op::Lbra | Op::Rts | Op::Rtn | Op::Rti => {
                known = Known::default();
            }
        }
    }
    let rewritten = !rewrite.is_empty();
    for (i, inst) in rewrite {
        insts[i] = inst;
    }
    let mut i = 0;
    insts.retain(|_| {
        i += 1;
        !delete[i - 1]
    });
    rewritten || delete.contains(&true)
}

static NEXT_CARRY_LABEL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `lda m / adc #0 / sta m` (the top of a multi-byte add) is `bcc + / inc m`,
/// and `lda m / sbc #0 / sta m` is `bcs + / dec m`, when A and the flags are
/// not needed afterwards. Only for compiler-owned memory: an I/O register
/// would see a different access pattern.
fn carry_increments(insts: &mut Vec<Inst>, returns_carry: bool) -> bool {
    let live = live_after(insts, returns_carry);
    let mut i = 0;
    let mut changed = false;
    while i + 2 < insts.len() {
        let replacement = match (&insts[i], &insts[i + 1], &insts[i + 2]) {
            (
                Inst::Op(Op::Lda, load),
                Inst::Op(op @ (Op::Adc | Op::Sbc), Operand::Immediate(zero)),
                Inst::Op(Op::Sta, store),
            ) if zero == "$00"
                && load == store
                && matches!(load, Operand::Base(symbol) | Operand::Absolute(symbol) if is_compiler_symbol(symbol))
                && live[i + 2] & (A | NZ | C) == 0 =>
            {
                let (skip, step) = if *op == Op::Adc {
                    (Op::Bcc, Op::Inc)
                } else {
                    (Op::Bcs, Op::Dec)
                };
                let label = format!(
                    "@m65_carry_{}",
                    NEXT_CARRY_LABEL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                Some([
                    Inst::op(skip, Operand::Label(label.clone())),
                    Inst::op(step, load.clone()),
                    Inst::Label(label),
                ])
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            insts.splice(i..i + 3, replacement);
            changed = true;
            i += 1;
            continue;
        }
        // `sta t / clc / lda m / adc t` (t dead after) is `clc / adc m`: A
        // already holds t, and adding commutes. Likewise and/ora/eor.
        if i + 3 < insts.len() {
            let commuted = match (&insts[i], &insts[i + 1], &insts[i + 2], &insts[i + 3]) {
                (
                    Inst::Op(Op::Sta, Operand::Base(t)),
                    clc @ Inst::Op(Op::Clc, _),
                    Inst::Op(Op::Lda, m),
                    Inst::Op(Op::Adc, Operand::Base(t2)),
                ) if t == t2
                    && runtime_byte(t).is_some_and(|bit| live[i + 3] & bit == 0)
                    && !matches!(m, Operand::Base(other) if other == t) =>
                {
                    Some(vec![clc.clone(), Inst::op(Op::Adc, m.clone())])
                }
                _ => None,
            };
            if let Some(commuted) = commuted {
                insts.splice(i..i + 4, commuted);
                changed = true;
                i += 1;
                continue;
            }
        }
        if i + 2 < insts.len() {
            let commuted = match (&insts[i], &insts[i + 1], &insts[i + 2]) {
                (
                    Inst::Op(Op::Sta, Operand::Base(t)),
                    Inst::Op(Op::Lda, m),
                    Inst::Op(op @ (Op::And | Op::Ora | Op::Eor), Operand::Base(t2)),
                ) if t == t2
                    && runtime_byte(t).is_some_and(|bit| live[i + 2] & bit == 0)
                    && !matches!(m, Operand::Base(other) if other == t)
                    && !matches!(
                        m,
                        Operand::IndirectZ(_) | Operand::FlatZ(_) | Operand::IndirectY(_)
                    ) =>
                {
                    Some(vec![Inst::op(*op, m.clone())])
                }
                _ => None,
            };
            if let Some(commuted) = commuted {
                insts.splice(i..i + 3, commuted);
                changed = true;
            }
        }
        i += 1;
    }
    changed
}

/// (increment, decrement) for an index-register load.
fn step_ops(load: Op) -> Option<(Op, Op)> {
    match load {
        Op::Ldx => Some((Op::Inx, Op::Dex)),
        Op::Ldy => Some((Op::Iny, Op::Dey)),
        Op::Ldz => Some((Op::Inz, Op::Dez)),
        _ => None,
    }
}

fn parse_byte(text: &str) -> Option<u8> {
    u8::from_str_radix(text.strip_prefix('$')?, 16).ok()
}

// ---- jumps ------------------------------------------------------------------

/// Whether `label` is defined among the labels starting at `from`.
fn label_follows(insts: &[Inst], from: usize, label: &str) -> bool {
    insts[from..]
        .iter()
        .take_while(|inst| matches!(inst, Inst::Label(_)))
        .any(|inst| matches!(inst, Inst::Label(name) if name == label))
}

/// Where a jump to `label` really ends up, following unconditional branches.
/// Cheap-local (`@`) labels are never substituted: their scope is local.
fn final_target(insts: &[Inst], labels: &HashMap<String, usize>, label: &str) -> String {
    let mut current = label.to_owned();
    for _ in 0..8 {
        let Some(&at) = labels.get(&current) else {
            break;
        };
        let next = insts[at..]
            .iter()
            .find(|inst| !matches!(inst, Inst::Label(_)));
        match next {
            Some(Inst::Op(op, Operand::Label(target)))
                if op.is_unconditional_branch()
                    && !target.starts_with('@')
                    && *target != current =>
            {
                current = target.clone();
            }
            _ => break,
        }
    }
    current
}

fn clean_jumps(insts: &mut Vec<Inst>) -> bool {
    let mut changed = false;
    // Jumps to unconditional jumps go straight to the final target.
    let labels = label_positions(insts);
    let mut retargets = Vec::new();
    for (i, inst) in insts.iter().enumerate() {
        if let Some(target) = branch_target(inst) {
            let final_label = final_target(insts, &labels, target);
            if final_label != target {
                retargets.push((i, final_label));
            }
        }
    }
    for (i, label) in retargets {
        if let Inst::Op(_, operand) = &mut insts[i] {
            *operand = Operand::Label(label);
            changed = true;
        }
    }

    let mut out = Vec::with_capacity(insts.len());
    let mut i = 0;
    let mut unreachable = false;
    while i < insts.len() {
        let inst = &insts[i];
        if matches!(inst, Inst::Label(_)) {
            unreachable = false;
            out.push(inst.clone());
            i += 1;
            continue;
        }
        if unreachable {
            changed = true;
            i += 1;
            continue;
        }
        if let Some(target) = branch_target(inst) {
            // A branch to the very next instruction does nothing.
            if label_follows(insts, i + 1, target) {
                changed = true;
                i += 1;
                continue;
            }
            // `bcond L1 / bra L2 / L1:` is `b!cond L2 / L1:`.
            if let (Inst::Op(op, _), Some(Inst::Op(next, Operand::Label(far)))) =
                (inst, insts.get(i + 1))
            {
                if let (Some((_, _, Some(inverse))), true) =
                    (op.branch_forms(), next.is_unconditional_branch())
                {
                    if label_follows(insts, i + 2, target) {
                        out.push(Inst::op(inverse, Operand::Label(far.clone())));
                        changed = true;
                        i += 2;
                        continue;
                    }
                }
            }
        }
        if let Inst::Op(op, _) = inst {
            unreachable = op.ends_flow();
        }
        out.push(inst.clone());
        i += 1;
    }
    *insts = out;
    changed
}

/// Uses short branches wherever the target is within reach. Starts with
/// every branch short and lengthens the ones out of range until stable.
fn relax_branches(insts: &mut [Inst]) {
    let mut long = vec![false; insts.len()];
    for (i, inst) in insts.iter().enumerate() {
        long[i] = branch_target(inst).is_some_and(|target| {
            !insts
                .iter()
                .any(|inst| matches!(inst, Inst::Label(name) if name == target))
        });
    }
    loop {
        let mut address = Vec::with_capacity(insts.len() + 1);
        let mut at: i64 = 0;
        for (i, inst) in insts.iter().enumerate() {
            address.push(at);
            at += match inst {
                Inst::Op(op, _) if op.branch_forms().is_some() => {
                    if long[i] {
                        3
                    } else {
                        2
                    }
                }
                _ => i64::from(inst.size().expect("raw assembly is never relaxed")),
            };
        }
        let labels = label_positions(insts);
        let mut changed = false;
        for (i, inst) in insts.iter().enumerate() {
            if long[i] {
                continue;
            }
            let Some(target) = branch_target(inst) else {
                continue;
            };
            let distance = address[labels[target]] - (address[i] + 2);
            if !(-128..=127).contains(&distance) {
                long[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (i, inst) in insts.iter_mut().enumerate() {
        if let Inst::Op(op, Operand::Label(_)) = inst {
            if let Some((short, long_form, _)) = op.branch_forms() {
                *op = if long[i] { long_form } else { short };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(op: Op, operand: Operand) -> Inst {
        Inst::op(op, operand)
    }

    fn base(symbol: &str) -> Operand {
        Operand::Base(symbol.to_owned())
    }

    fn imm(text: &str) -> Operand {
        Operand::Immediate(text.to_owned())
    }

    fn label(name: &str) -> Operand {
        Operand::Label(name.to_owned())
    }

    #[test]
    fn reload_after_store_disappears() {
        let insts = vec![
            op(Op::Jsr, label("f")),
            op(Op::Sta, base(ACC_SYMBOL)),
            op(Op::Lda, base(ACC_SYMBOL)),
            op(Op::Cmp, imm("$00")),
            op(Op::Lbne, label("done")),
            op(Op::Lda, imm("$01")),
            Inst::Label("done".to_owned()),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert_eq!(
            out,
            vec![
                op(Op::Jsr, label("f")),
                op(Op::Cmp, imm("$00")),
                op(Op::Bne, label("done")),
                op(Op::Lda, imm("$01")),
                Inst::Label("done".to_owned()),
                Inst::implied(Op::Rts),
            ]
        );
    }

    #[test]
    fn zero_compare_stays_when_carry_is_read() {
        let insts = vec![
            op(Op::Lda, Operand::Absolute("value".to_owned())),
            op(Op::Cmp, imm("$00")),
            op(Op::Lbcs, label("done")),
            op(Op::Lda, imm("$01")),
            Inst::Label("done".to_owned()),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert!(out.contains(&op(Op::Cmp, imm("$00"))));
    }

    #[test]
    fn user_memory_is_never_assumed_unchanged() {
        // `control` may be a hardware register whose reads differ from writes.
        let control = Operand::Absolute("control".to_owned());
        let insts = vec![
            op(Op::Lda, imm("$02")),
            op(Op::Sta, control.clone()),
            op(Op::Lda, control.clone()),
            op(Op::Sta, control.clone()),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts.clone(), true);
        assert_eq!(out, insts);
    }

    #[test]
    fn calls_to_flag_routines_drop_the_zero_compare() {
        let insts = vec![
            op(Op::Jsr, label("f")),
            op(Op::Cmp, imm("$00")),
            op(Op::Lbeq, label("out")),
            op(Op::Jsr, label("g")),
            op(Op::Cmp, imm("$00")),
            op(Op::Lbeq, label("out")),
            Inst::Label("out".to_owned()),
            Inst::implied(Op::Rts),
        ];
        let flag_routines = HashSet::from(["f".to_owned()]);
        let out = optimize_routine_with(insts, true, &flag_routines, false);
        let compares = out
            .iter()
            .filter(|inst| matches!(inst, Inst::Op(Op::Cmp, _)))
            .count();
        assert_eq!(compares, 1, "only g's result needs a compare: {out:?}");
    }

    #[test]
    fn repeated_frame_offsets_reuse_y() {
        let frame = Operand::IndirectY(FP_SYMBOL.to_owned());
        let insts = vec![
            op(Op::Ldy, imm("$03")),
            op(Op::Sta, frame.clone()),
            op(Op::Ldy, imm("$03")),
            op(Op::Lda, frame.clone()),
            Inst::implied(Op::Pha),
            op(Op::Jsr, label("f")),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert_eq!(
            out,
            vec![
                op(Op::Ldy, imm("$03")),
                op(Op::Sta, frame),
                Inst::implied(Op::Pha),
                op(Op::Jsr, label("f")),
                Inst::implied(Op::Rts),
            ]
        );
    }

    #[test]
    fn neighbouring_frame_offsets_step_y() {
        let frame = Operand::IndirectY(FP_SYMBOL.to_owned());
        let insts = vec![
            op(Op::Ldy, imm("$0a")),
            op(Op::Lda, frame.clone()),
            Inst::implied(Op::Pha),
            op(Op::Ldy, imm("$09")),
            op(Op::Lda, frame.clone()),
            Inst::implied(Op::Pha),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert_eq!(out[3], Inst::implied(Op::Dey));
    }

    #[test]
    fn pointer_stores_forget_frame_contents() {
        let frame = Operand::IndirectY(FP_SYMBOL.to_owned());
        let insts = vec![
            op(Op::Ldy, imm("$03")),
            op(Op::Sta, frame.clone()),
            op(Op::Sta, Operand::IndirectZ(PTR_SYMBOL.to_owned())),
            op(Op::Lda, frame.clone()),
            Inst::implied(Op::Pha),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts.clone(), true);
        assert!(out.contains(&op(Op::Lda, frame)));
    }

    #[test]
    fn interrupt_handlers_keep_resetting_z() {
        let insts = vec![
            Inst::implied(Op::Phz),
            op(Op::Ldz, imm("$00")),
            op(Op::Stz, Operand::Absolute("flag".to_owned())),
            Inst::implied(Op::Plz),
            Inst::implied(Op::Rti),
        ];
        let out = optimize_routine(insts.clone(), false);
        assert_eq!(out, insts);
    }

    #[test]
    fn zero_compare_after_load_disappears() {
        let insts = vec![
            op(Op::Lda, Operand::Absolute("value".to_owned())),
            op(Op::Cmp, imm("$00")),
            op(Op::Lbeq, label("done")),
            op(Op::Lda, imm("$01")),
            Inst::Label("done".to_owned()),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert_eq!(out[1], op(Op::Beq, label("done")));
    }

    #[test]
    fn branch_over_jump_is_inverted() {
        let insts = vec![
            op(Op::Lda, Operand::Absolute("value".to_owned())),
            op(Op::Lbeq, label("skip")),
            op(Op::Lbra, label("far")),
            Inst::Label("skip".to_owned()),
            op(Op::Lda, imm("$01")),
            Inst::Label("far".to_owned()),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert_eq!(out[1], op(Op::Bne, label("far")));
        assert!(!out
            .iter()
            .any(|inst| matches!(inst, Inst::Op(Op::Lbra | Op::Bra, _))));
    }

    #[test]
    fn distant_branches_stay_long() {
        let mut insts = vec![op(Op::Lbne, label("far"))];
        insts.extend((0..50).map(|_| op(Op::Sta, Operand::Absolute("sink".to_owned()))));
        insts.push(Inst::Label("far".to_owned()));
        insts.push(Inst::implied(Op::Rts));
        let out = optimize_routine(insts, true);
        assert_eq!(out[0], op(Op::Lbne, label("far")));
    }
}
