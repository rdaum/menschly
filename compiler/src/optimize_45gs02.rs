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

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::lower_45gs02::{
    ACC_SYMBOL, ARG_SYMBOL, FARP_SYMBOL, FP_SYMBOL, LEAVE_ROUTINE, CLEAR_ROUTINES, DIVS_ROUTINE, PTR_SYMBOL,
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
/// The overflow flag.
const V: Locs = 1 << 30;
const ALL: Locs = (1 << 31) - 1;

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
    optimize_routine_with(insts, z_zero_on_entry, &HashSet::new(), false, None, &HashSet::new())
}

/// What a routine's return hands back, for liveness at RTS: the carry (a
/// carry result), and how many bytes of result (`None`: not known, so all
/// the result registers count as read).
#[derive(Clone, Copy)]
struct Returns {
    carry: bool,
    bytes: Option<u16>,
}

impl Returns {
    /// The registers and flags a return reads. Results are in A/X/Y, a
    /// 32-bit result's top byte in ACC+3; N/Z describe A for 1- and 2-byte
    /// results.
    fn uses(self) -> Locs {
        let result = match self.bytes {
            None => A | X | Y | ACC3 | NZ,
            Some(0) => 0,
            Some(1) => A | NZ,
            Some(2) => A | X | NZ,
            Some(3) => A | X | Y,
            Some(_) => A | X | Y | ACC3,
        };
        result | if self.carry { C } else { 0 }
    }
}

/// Like [`optimize_routine`]; `flag_routines` are routines known to return
/// with N/Z describing A (see `lower_45gs02`), so a `CMP #0` of their
/// result can go.
pub fn optimize_routine_with(
    insts: Vec<Inst>,
    z_zero_on_entry: bool,
    flag_routines: &HashSet<String>,
    returns_carry: bool,
    result_bytes: Option<u16>,
    plain_memory: &HashSet<String>,
) -> Vec<Inst> {
    let returns = Returns {
        carry: returns_carry,
        bytes: result_bytes,
    };
    // Inline assembly is an opaque step (see `effect`), unless it names one
    // of the routine's labels: then control may pass between it and the
    // compiled code in ways the analysis cannot see.
    if names_a_label(&insts) {
        return insts;
    }
    let mut insts = insts;
    loop {
        let before = insts.len();
        let mut changed = clean_jumps(&mut insts);
        changed |= eliminate_dead_code(&mut insts, returns);
        changed |= eliminate_dead_temporaries(&mut insts);
        changed |=
            eliminate_redundant(&mut insts, z_zero_on_entry, flag_routines, returns, plain_memory);
        changed |= carry_increments(&mut insts, returns, plain_memory);
        changed |= loop_counters_in_x(&mut insts, returns);
        changed |= overwritten_variable_stores(&mut insts, plain_memory);
        changed |= merge_interrupt_holds(&mut insts, returns);
        if !changed && insts.len() == before {
            break;
        }
    }
    // Last, once known bytes have folded away: what is left of 4-byte
    // copies and adds of memory goes 32 bits at a time.
    quad_runs(&mut insts, returns, z_zero_on_entry, plain_memory);
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

/// The runtime-register bytes a 32-bit access at `operand` touches.
fn quad_locs(operand: &Operand) -> Locs {
    match operand {
        Operand::Base(symbol) => runtime_bytes(symbol, 4),
        _ => 0,
    }
}

/// Whether any inline assembly mentions one of the routine's labels.
fn names_a_label(insts: &[Inst]) -> bool {
    let labels: Vec<&str> = insts
        .iter()
        .filter_map(|inst| match inst {
            Inst::Label(label) => Some(label.as_str()),
            _ => None,
        })
        .collect();
    insts.iter().any(|inst| match inst {
        Inst::Raw(text) => text
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '@'))
            .any(|word| labels.contains(&word)),
        _ => false,
    })
}

/// The most bytes a piece of inline assembly can take: 5 an instruction
/// (a 32-bit form with an absolute operand), none for labels, comments and
/// directives.
fn raw_size_bound(text: &str) -> u16 {
    let instructions = text
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && !line.ends_with(':') && !line.starts_with(';') && !line.starts_with('.')
        })
        .count();
    u16::try_from(instructions * 5).unwrap_or(u16::MAX)
}

fn effect(inst: &Inst) -> Effect {
    let none = Effect {
        uses: 0,
        defs: 0,
        removable: false,
    };
    // Inline assembly may read and change anything (it leaves Z zero, but
    // liveness need not know that).
    if matches!(inst, Inst::Raw(_)) {
        return Effect {
            uses: ALL,
            defs: ALL,
            removable: false,
        };
    }
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
        // The 32-bit forms: A, X, Y and Z hold the low to high bytes.
        Op::Ldq | Op::Adcq => {
            let bytes = quad_locs(operand);
            let carry = if op == Op::Adcq { A | X | Y | Z | C } else { 0 };
            (
                address_uses(operand) | bytes | carry,
                A | X | Y | Z | NZ | if op == Op::Adcq { C } else { 0 },
                pure_read(operand),
            )
        }
        Op::Stq => {
            let target = quad_locs(operand);
            (
                A | X | Y | Z | address_uses(operand),
                target,
                target.count_ones() == 4,
            )
        }
        Op::Php => (NZ | C | V, 0, false),
        Op::Plp => (0, NZ | C | V, false),
        Op::Sei => (0, 0, false),
        Op::Bit => (A | read, NZ | V, pure_read(operand)),
        Op::Pha | Op::Phx | Op::Phy | Op::Phz => (register_bit(op), 0, false),
        Op::Pla | Op::Plx | Op::Ply | Op::Plz => (0, register_bit(op) | NZ, false),
        Op::Bne | Op::Beq | Op::Lbne | Op::Lbeq => (NZ, 0, false),
        Op::Bmi | Op::Bpl | Op::Lbmi | Op::Lbpl => (NZ, 0, false),
        Op::Bvs | Op::Bvc | Op::Lbvs | Op::Lbvc => (V, 0, false),
        Op::Bcc | Op::Bcs | Op::Lbcc | Op::Lbcs => (C, 0, false),
        Op::Bra | Op::Lbra => (0, 0, false),
        // These only zero the math unit's inputs (with Z).
        Op::Jsr if matches!(operand, Operand::Label(name) if CLEAR_ROUTINES.contains(&name.as_str())) => {
            (Z, 0, false)
        }
        // The signed divide takes ACC and ARG (and the width in X), leaving
        // the temp registers alone.
        Op::Jsr if matches!(operand, Operand::Label(name) if name == DIVS_ROUTINE) => (
            A | X | Y | Z | FP | ACC | ARG,
            A | X | Y | NZ | C | ACC | FARP | ARG,
            false,
        ),
        // Callees take their arguments on the stack or in their own
        // parameter storage, never in the base-page scratch registers; they
        // return results in A/X/Y (and ACC+3), keep Z = 0 and the frame
        // pointer, and clobber the scratch registers. An indirect call reads
        // its vector.
        Op::Jsr => {
            let arguments = match operand {
                Operand::Indirect(vector) => runtime_bytes(vector, 2),
                _ => 0,
            };
            let clobbers = match operand {
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
/// `returns`: what the routine's returns hand back.
fn live_after(insts: &[Inst], returns: Returns) -> Vec<Locs> {
    let labels = label_positions(insts);
    let mut effects: Vec<Effect> = insts.iter().map(effect).collect();
    // A return reads the result registers and the frame; Z = 0 and the
    // frame pointer are kept for the caller.
    for (inst, effect) in insts.iter().zip(&mut effects) {
        if matches!(inst, Inst::Op(Op::Rts | Op::Rtn, _)) {
            effect.uses = Z | FP | returns.uses();
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

// ---- overwritten stores to variables ----------------------------------------

/// Deletes a store to one of the module's ordinary variables that is
/// stored again, before anything could read it, in the same straight run
/// of code (no label, branch, call or return between). Anything that might
/// read it ends the wait: a read of that byte or (indexed) of that variable,
/// any access through a pointer, a store elsewhere that may be hardware
/// (DMA reads memory), inline assembly.
/// `plp / php / sei` between two stretches that hold interrupts off (two
/// uses of the math unit) is dropped when the flags `plp` restores are not
/// read: interrupts stay off across both, and the second stretch's `plp`
/// pulls what the first pushed.
fn merge_interrupt_holds(insts: &mut Vec<Inst>, returns: Returns) -> bool {
    let held = |at: usize| {
        matches!(
            insts.get(at..at + 3),
            Some([Inst::Op(Op::Plp, _), Inst::Op(Op::Php, _), Inst::Op(Op::Sei, _)])
        )
    };
    if !(0..insts.len()).any(held) {
        return false;
    }
    let live = live_after(insts, returns);
    let mut delete = vec![false; insts.len()];
    let mut i = 0;
    while i < insts.len() {
        if held(i) && live[i + 2] & (NZ | C | V) == 0 {
            delete[i..i + 3].fill(true);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut k = 0;
    insts.retain(|_| {
        k += 1;
        !delete[k - 1]
    });
    delete.contains(&true)
}

fn overwritten_variable_stores(insts: &mut Vec<Inst>, plain_memory: &HashSet<String>) -> bool {
    if plain_memory.is_empty() {
        return false;
    }
    let plain_byte = |operand: &Operand| match operand {
        Operand::Base(symbol) | Operand::Absolute(symbol)
            if plain_memory.contains(split_offset(symbol).0) =>
        {
            Some(symbol.clone())
        }
        _ => None,
    };
    let mut waiting: HashMap<String, usize> = HashMap::new();
    let mut dead = vec![false; insts.len()];
    for (i, inst) in insts.iter().enumerate() {
        let Inst::Op(op, operand) = inst else {
            waiting.clear();
            continue;
        };
        if op.branch_forms().is_some() || matches!(op, Op::Jsr | Op::Rts | Op::Rtn | Op::Rti) {
            waiting.clear();
            continue;
        }
        let stores = matches!(op, Op::Sta | Op::Stx | Op::Sty | Op::Stz);
        match operand {
            Operand::Base(symbol) | Operand::Absolute(symbol) => {
                if let Some(byte) = plain_byte(operand) {
                    if stores {
                        if let Some(earlier) = waiting.insert(byte, i) {
                            dead[earlier] = true;
                        }
                    } else {
                        // A read (or read-modify-write) of the byte.
                        waiting.remove(&byte);
                    }
                } else if stores && !is_compiler_symbol(symbol) {
                    // May be hardware that reads memory.
                    waiting.clear();
                }
            }
            Operand::AbsoluteX(symbol) => {
                let name = split_offset(symbol).0;
                waiting.retain(|byte, _| split_offset(byte).0 != name);
                if stores && !plain_memory.contains(name) && !is_compiler_symbol(symbol) {
                    waiting.clear();
                }
            }
            Operand::IndirectY(_) | Operand::IndirectZ(_) | Operand::FlatZ(_) | Operand::Indirect(_) => {
                waiting.clear();
            }
            _ => {}
        }
    }
    if !dead.contains(&true) {
        return false;
    }
    let mut i = 0;
    insts.retain(|_| {
        i += 1;
        !dead[i - 1]
    });
    true
}

// ---- loop counters in X -----------------------------------------------------

/// Keeps a byte of the routine's own storage (a local or temporary) in X
/// for a loop: when all its uses lie between its first store before the
/// loop's label and the branch back (so it is dead after), nothing else
/// there uses or changes X, there are no calls or inline assembly, nothing
/// jumps in from outside, and it is only loaded, stored, cleared, stepped,
/// or loaded into X for indexing. Its loads become TXA, its stores TAX, its
/// steps INX/DEX, and its loads into X go; `txa / cmp #n` becomes
/// `cpx #n` when A is not needed after. One byte a pass.
fn loop_counters_in_x(insts: &mut Vec<Inst>, returns: Returns) -> bool {
    if insts.iter().any(|inst| matches!(inst, Inst::Raw(_))) {
        return false;
    }
    let live = live_after(insts, returns);
    let labels = label_positions(insts);
    // Each candidate byte's uses; names used wider, or through, are out.
    let mut uses: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut out = HashSet::new();
    for (i, inst) in insts.iter().enumerate() {
        let Inst::Op(_, operand) = inst else { continue };
        match operand {
            Operand::Base(symbol) | Operand::Absolute(symbol) => {
                let (name, offset) = split_offset(symbol);
                if offset != Some(0) {
                    out.insert(name.to_owned());
                } else if is_temporary(name) {
                    uses.entry(name.to_owned()).or_default().push(i);
                }
            }
            Operand::IndirectY(pointer)
            | Operand::IndirectZ(pointer)
            | Operand::FlatZ(pointer)
            | Operand::AbsoluteX(pointer) => {
                out.insert(split_offset(pointer).0.to_owned());
            }
            _ => {}
        }
    }
    for (back, inst) in insts.iter().enumerate() {
        let Some(target) = branch_target(inst) else { continue };
        let Some(&head) = labels.get(target) else { continue };
        if head >= back {
            continue;
        }
        for (name, at) in &uses {
            if out.contains(name) {
                continue;
            }
            let (first, last) = (at[0], at[at.len() - 1]);
            if first > head || last > back || last < head {
                continue;
            }
            if let Some(rewrites) = counter_rewrites(insts, &live, &labels, first, back, at) {
                for (i, replacement) in rewrites.into_iter().rev() {
                    match replacement {
                        Some(inst) => insts[i] = inst,
                        None => {
                            insts.remove(i);
                        }
                    }
                }
                return true;
            }
        }
    }
    false
}

/// The rewrites keeping the byte used at `at` in X over `first..=back`, or
/// `None` when that is not safe.
fn counter_rewrites(
    insts: &[Inst],
    live: &[Locs],
    labels: &HashMap<String, usize>,
    first: usize,
    back: usize,
    at: &[usize],
) -> Option<Vec<(usize, Option<Inst>)>> {
    // Nothing jumps into the stretch from outside it.
    for (i, inst) in insts.iter().enumerate() {
        if (first..=back).contains(&i) {
            continue;
        }
        if let Some(target) = branch_target(inst) {
            if labels.get(target).is_some_and(|&to| to > first && to <= back) {
                return None;
            }
        }
    }
    let mut rewrites = Vec::new();
    for i in first..=back {
        let Inst::Op(op, operand) = &insts[i] else { continue };
        let flags_dead = live[i] & NZ == 0;
        if at.contains(&i) {
            rewrites.push((
                i,
                match op {
                    Op::Lda => Some(Inst::implied(Op::Txa)),
                    Op::Sta if flags_dead => Some(Inst::implied(Op::Tax)),
                    Op::Stz if flags_dead => Some(Inst::op(Op::Ldx, Operand::imm_byte(0))),
                    Op::Inc => Some(Inst::implied(Op::Inx)),
                    Op::Dec => Some(Inst::implied(Op::Dex)),
                    Op::Ldx if flags_dead => None,
                    _ => return None,
                },
            ));
            continue;
        }
        if matches!(op, Op::Jsr) {
            return None;
        }
        // Only indexing may read X, and nothing else may change it.
        let effect = effect(&insts[i]);
        if effect.defs & X != 0 || (effect.uses & X != 0 && !matches!(operand, Operand::AbsoluteX(_))) {
            return None;
        }
    }
    // `txa / cmp #n` with A dead after: `cpx #n`.
    let mut compares = Vec::new();
    for (k, (i, replacement)) in rewrites.iter().enumerate() {
        if *replacement != Some(Inst::implied(Op::Txa)) {
            continue;
        }
        if let Some(Inst::Op(Op::Cmp, immediate @ Operand::Immediate(_))) = insts.get(i + 1) {
            if live[i + 1] & A == 0 {
                compares.push((k, i + 1, immediate.clone()));
            }
        }
    }
    for (k, compare_at, immediate) in compares.into_iter().rev() {
        rewrites[k].1 = Some(Inst::op(Op::Cpx, immediate));
        rewrites.push((compare_at, None));
    }
    rewrites.sort_by_key(|(i, _)| *i);
    Some(rewrites)
}

// ---- 32-bit runs -----------------------------------------------------------

/// Byte `k` of a 4-byte value whose first byte is `operand`, when it is
/// plain memory: the compiler's own, or one of `plain_memory` (a module's
/// ordinary variables; never a fixed address, which may be hardware that
/// must see the accesses the source asks for).
fn quad_byte(operand: &Operand, k: u32, plain_memory: &HashSet<String>) -> Option<Operand> {
    let (Operand::Base(symbol) | Operand::Absolute(symbol)) = operand else {
        return None;
    };
    let (name, offset) = split_offset(symbol);
    if !is_compiler_symbol(name) && !plain_memory.contains(name) {
        return None;
    }
    let at = offset? + k;
    let byte = if at == 0 {
        name.to_owned()
    } else {
        format!("{name}+{at}")
    };
    Some(match operand {
        Operand::Base(_) => Operand::Base(byte),
        _ => Operand::Absolute(byte),
    })
}

/// Whether the four bytes from `a` and from `b` (same symbol) overlap.
fn quads_overlap(a: &Operand, b: &Operand) -> bool {
    let (Operand::Base(x) | Operand::Absolute(x), Operand::Base(y) | Operand::Absolute(y)) = (a, b)
    else {
        return true;
    };
    let ((xn, xo), (yn, yo)) = (split_offset(x), split_offset(y));
    xn == yn && xo.zip(yo).is_none_or(|(xo, yo)| xo.abs_diff(yo) < 4)
}

/// Rewrites 4-byte copies (`lda s / sta d` four times) as `ldq / stq` and
/// 4-byte adds (`clc` then `lda a / adc b / sta d` four times) as `clc /
/// ldq / adcq / stq`, each followed by `ldz #0`: when the bytes are plain
/// memory in a row, Z is zero before (so restoring it changes nothing),
/// and A, X, Y and N/Z are dead after (the Q forms leave other values).
fn quad_runs(
    insts: &mut Vec<Inst>,
    returns: Returns,
    z_zero_on_entry: bool,
    plain_memory: &HashSet<String>,
) {
    let live = live_after(insts, returns);
    let referenced: HashSet<String> = insts
        .iter()
        .filter_map(branch_target)
        .map(str::to_owned)
        .collect();
    let mut out = Vec::with_capacity(insts.len());
    let mut z_zero = z_zero_on_entry;
    let mut z_established = z_zero_on_entry;
    let mut i = 0;
    let operand_of = |inst: &Inst, want: Op| match inst {
        Inst::Op(op, operand) if *op == want => Some(operand.clone()),
        _ => None,
    };
    while i < insts.len() {
        let dead = |at: usize| live[at] & (A | X | Y | NZ) == 0;
        // A copy.
        if z_zero && i + 8 <= insts.len() && dead(i + 7) {
            let first = (operand_of(&insts[i], Op::Lda), operand_of(&insts[i + 1], Op::Sta));
            if let (Some(source), Some(target)) = first {
                let run = (0..4).all(|k| {
                    let k32 = k as u32;
                    quad_byte(&source, k32, plain_memory).is_some_and(|byte| {
                        operand_of(&insts[i + 2 * k], Op::Lda) == Some(byte)
                    }) && quad_byte(&target, k32, plain_memory).is_some_and(|byte| {
                        operand_of(&insts[i + 2 * k + 1], Op::Sta) == Some(byte)
                    })
                });
                if run && !quads_overlap(&source, &target) {
                    out.push(Inst::op(Op::Ldq, source));
                    out.push(Inst::op(Op::Stq, target));
                    out.push(Inst::op(Op::Ldz, Operand::imm_byte(0)));
                    i += 8;
                    continue;
                }
            }
        }
        // An add.
        if z_zero
            && i + 13 <= insts.len()
            && matches!(insts[i], Inst::Op(Op::Clc, _))
            && dead(i + 12)
        {
            let first = (
                operand_of(&insts[i + 1], Op::Lda),
                operand_of(&insts[i + 2], Op::Adc),
                operand_of(&insts[i + 3], Op::Sta),
            );
            if let (Some(left), Some(right), Some(target)) = first {
                let run = (0..4).all(|k| {
                    let (k32, at) = (k as u32, i + 1 + 3 * k);
                    quad_byte(&left, k32, plain_memory)
                        .is_some_and(|byte| operand_of(&insts[at], Op::Lda) == Some(byte))
                        && quad_byte(&right, k32, plain_memory)
                            .is_some_and(|byte| operand_of(&insts[at + 1], Op::Adc) == Some(byte))
                        && quad_byte(&target, k32, plain_memory)
                            .is_some_and(|byte| operand_of(&insts[at + 2], Op::Sta) == Some(byte))
                });
                if run {
                    out.push(Inst::implied(Op::Clc));
                    out.push(Inst::op(Op::Ldq, left));
                    out.push(Inst::op(Op::Adcq, right));
                    out.push(Inst::op(Op::Stq, target));
                    out.push(Inst::op(Op::Ldz, Operand::imm_byte(0)));
                    i += 13;
                    continue;
                }
            }
        }
        // Z: zero at branch targets once established (the codegen
        // invariant), after LDZ #0; anything else that sets it, unknown.
        match &insts[i] {
            Inst::Label(label) if referenced.contains(label) => z_zero = z_established,
            Inst::Op(Op::Ldz, operand) => {
                z_zero = operand.is_zero_immediate();
                z_established |= z_zero;
            }
            Inst::Op(Op::Inz | Op::Dez | Op::Plz | Op::Ldq | Op::Adcq, _) => z_zero = false,
            Inst::Op(Op::Jsr, _) | Inst::Raw(_) => z_zero = true,
            _ => {}
        }
        out.push(insts[i].clone());
        i += 1;
    }
    *insts = out;
}

// ---- dead stores to temporaries -----------------------------------------

/// Whether `symbol` (with any offset) is storage whose value matters only
/// within the routine that writes it: the scratch slots and expression
/// temporaries, and locals (static or in the direct page), which start
/// undefined and whose address Mensch never takes. (Not parameters: a
/// caller writes its callee's.)
fn is_temporary(symbol: &str) -> bool {
    let (name, _) = split_offset(symbol);
    name.starts_with("__static_")
        || (name.starts_with("__dp_")
            && (name.contains("_scratch") || name.contains("_temp_") || name.contains("_local_")))
}

/// The key of one byte of a temporary: "name+offset".
fn temporary_byte(symbol: &str, extra: u32) -> Option<String> {
    let (name, offset) = split_offset(symbol);
    if !is_temporary(symbol) {
        return None;
    }
    Some(format!("{name}+{}", offset? + extra))
}

/// The temporary bytes an instruction reads, and the one it overwrites;
/// `None` for reads it cannot pin down (a call may read any of them).
fn temporary_effect(inst: &Inst) -> (Option<Vec<String>>, Option<String>) {
    let Inst::Op(op, operand) = inst else {
        // Inline assembly may read any of them.
        return (if matches!(inst, Inst::Raw(_)) { None } else { Some(Vec::new()) }, None);
    };
    if *op == Op::Jsr {
        return (None, None);
    }
    if *op == Op::Stq {
        // A 32-bit store's four bytes are left live (never deleted here).
        return (Some(Vec::new()), None);
    }
    let stores = matches!(op, Op::Sta | Op::Stx | Op::Sty | Op::Stz);
    match operand {
        Operand::Base(symbol) | Operand::Absolute(symbol) => {
            if stores {
                (Some(Vec::new()), temporary_byte(symbol, 0))
            } else {
                let bytes = match op {
                    Op::Inw | Op::Dew => 2,
                    Op::Ldq | Op::Adcq => 4,
                    _ => 1,
                };
                (Some((0..bytes).filter_map(|i| temporary_byte(symbol, i)).collect()), None)
            }
        }
        Operand::IndirectY(pointer) | Operand::IndirectZ(pointer) => {
            (Some((0..2).filter_map(|i| temporary_byte(pointer, i)).collect()), None)
        }
        Operand::FlatZ(pointer) => {
            (Some((0..4).filter_map(|i| temporary_byte(pointer, i)).collect()), None)
        }
        Operand::AbsoluteX(symbol) if is_temporary(symbol) => (None, None),
        _ => (Some(Vec::new()), None),
    }
}

/// Deletes stores to temporaries that nothing reads before the byte is
/// written again or the routine returns.
fn eliminate_dead_temporaries(insts: &mut Vec<Inst>) -> bool {
    let effects: Vec<_> = insts.iter().map(temporary_effect).collect();
    if !effects.iter().any(|(_, def)| def.is_some()) {
        return false;
    }
    let every: HashSet<String> = effects
        .iter()
        .flat_map(|(uses, def)| uses.iter().flatten().chain(def.iter()).cloned())
        .collect();
    let labels = label_positions(insts);
    let n = insts.len();
    // Falling off the end leaves everything live.
    let mut live_in: Vec<HashSet<String>> = vec![HashSet::new(); n + 1];
    live_in[n] = every.clone();
    let mut live_out: Vec<HashSet<String>> = vec![HashSet::new(); n];
    loop {
        let mut changed = false;
        for i in (0..n).rev() {
            let (falls_through, returns) = match &insts[i] {
                Inst::Op(op, _) => (
                    !op.ends_flow(),
                    matches!(op, Op::Rts | Op::Rtn | Op::Rti),
                ),
                _ => (true, false),
            };
            let mut out = if falls_through && !returns {
                live_in[i + 1].clone()
            } else {
                HashSet::new()
            };
            if let Some(target) = branch_target(&insts[i]) {
                match labels.get(target) {
                    Some(&at) => out.extend(live_in[at].iter().cloned()),
                    None => out.extend(every.iter().cloned()),
                }
            }
            let (uses, def) = &effects[i];
            let mut input = out.clone();
            if let Some(def) = def {
                input.remove(def);
            }
            match uses {
                Some(uses) => input.extend(uses.iter().cloned()),
                None => input.extend(every.iter().cloned()),
            }
            if out != live_out[i] || input != live_in[i] {
                live_out[i] = out;
                live_in[i] = input;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut i = 0;
    let before = insts.len();
    insts.retain(|_| {
        i += 1;
        !matches!(&effects[i - 1].1, Some(def) if !live_out[i - 1].contains(def))
    });
    insts.len() != before
}

fn eliminate_dead_code(insts: &mut Vec<Inst>, returns: Returns) -> bool {
    let live = live_after(insts, returns);
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
    /// The carry flag, when known (after CLC, SEC, or a folded add).
    carry: Option<bool>,
    /// Memory known to hold a constant, by key: (key, immediate), from a
    /// store of a register that held the immediate. Kept after the
    /// register moves on, until the memory is written again.
    values: Vec<(String, String)>,
    /// The four bytes A, X, Y and Z together hold (after LDQ or STQ): the
    /// key of their first byte.
    quad: Option<String>,
    /// The module's ordinary variables (not fixed, not volatile): tracked
    /// like the compiler's own storage, on the understanding that they
    /// change only where the code writes them (see `clobber`).
    plain: std::rc::Rc<HashSet<String>>,
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
        self.quad = None;
        match key {
            Some(key) => self.regs[reg] = vec![key],
            None => self.forget(reg),
        }
    }

    /// The register now holds a new, unknown value.
    fn forget(&mut self, reg: usize) {
        self.quad = None;
        self.next_value += 1;
        self.regs[reg] = vec![format!("v:{}", self.next_value)];
    }

    /// The byte register `reg` is known to hold.
    fn byte_in(&self, reg: usize) -> Option<u8> {
        self.regs[reg]
            .iter()
            .find_map(|key| key.strip_prefix('#').and_then(parse_byte))
    }

    /// The byte an operand is known to read: an immediate, or memory known
    /// to hold a constant.
    fn byte_of(&self, operand: &Operand) -> Option<u8> {
        match operand {
            Operand::Immediate(text) => parse_byte(text),
            _ => self
                .key(operand)
                .and_then(|key| self.constant(&key))
                .and_then(|text| parse_byte(&text)),
        }
    }

    /// The register holding the value at `key`, other than `except`.
    fn holder(&self, key: &str, except: usize) -> Option<usize> {
        (0..4).find(|&reg| reg != except && self.holds(reg, key))
    }

    /// The immediate a key is known to equal (a register holds both, or
    /// the memory was last given a known byte).
    fn constant(&self, key: &str) -> Option<String> {
        self.regs
            .iter()
            .find_map(|keys| {
                keys.iter().any(|held| held == key).then(|| {
                    keys.iter()
                        .find_map(|held| held.strip_prefix('#').map(str::to_owned))
                })?
            })
            .or_else(|| {
                self.values
                    .iter()
                    .find(|(held, _)| held == key)
                    .map(|(_, value)| value.clone())
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
        self.values.retain(|(held, _)| keep(held));
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

    /// Whether `symbol` is one of the module's ordinary variables.
    fn is_plain(&self, symbol: &str) -> bool {
        self.plain.contains(split_offset(symbol).0)
    }

    /// Forgets what the module's ordinary variables hold.
    fn forget_plain(&mut self) {
        let plain = self.plain.clone();
        let is_plain = |key: &str| {
            key.strip_prefix("a:")
                .or_else(|| key.strip_prefix("z:"))
                .is_some_and(|symbol| plain.contains(split_offset(symbol).0))
        };
        self.retain_keys(|key| !is_plain(key));
        if self.quad.as_deref().is_some_and(is_plain) {
            self.quad = None;
        }
    }

    /// A key for the value an operand reads, if it is a known kind.
    fn key(&self, operand: &Operand) -> Option<String> {
        match operand {
            Operand::Immediate(text) => Some(format!("#{text}")),
            Operand::Base(symbol) if is_compiler_symbol(symbol) || self.is_plain(symbol) => {
                Some(format!("z:{symbol}"))
            }
            Operand::Absolute(symbol) if is_compiler_symbol(symbol) || self.is_plain(symbol) => {
                Some(format!("a:{symbol}"))
            }
            Operand::IndirectY(pointer) if pointer == FP_SYMBOL => self.frame_key(),
            _ => None,
        }
    }

    /// Forgets whatever memory a write to `operand` (of `bytes` bytes) may change.
    fn clobber(&mut self, operand: &Operand, bytes: u32) {
        // A write anywhere near the bytes the Q registers mirror ends that.
        if let (Some(quad), Operand::Base(symbol) | Operand::Absolute(symbol)) =
            (self.quad.as_ref(), operand)
        {
            let quad_name = split_offset(quad.split_once(':').map_or(quad.as_str(), |(_, s)| s)).0;
            if split_offset(symbol).0 == quad_name {
                self.quad = None;
            }
        } else if !matches!(operand, Operand::Base(_) | Operand::Absolute(_)) {
            self.quad = None;
        }
        match operand {
            Operand::Base(symbol) | Operand::Absolute(symbol)
                if is_compiler_symbol(symbol) || self.is_plain(symbol) =>
            {
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
            // Another user-named symbol (fixed, volatile, or another
            // module's) cannot alias compiler storage; but it may be
            // hardware, such as the DMA controller, that writes memory.
            Operand::Base(symbol) | Operand::Absolute(symbol) if !symbol.starts_with('$') => {
                self.forget_plain();
            }
            Operand::IndirectY(pointer) if pointer == FP_SYMBOL => match self.frame_key() {
                Some(key) => self.retain_keys(|held| held != key),
                None => self.retain_keys(|held| !held.starts_with("fp:")),
            },
            // Through a pointer: Mensch takes no address of compiler-owned
            // storage (locals, parameters, temporaries), so only the stack
            // frame may be reached.
            Operand::IndirectY(_) | Operand::IndirectZ(_) | Operand::FlatZ(_) => {
                self.retain_keys(|held| !held.starts_with("fp:"));
                // A pointer can reach any of the module's variables.
                self.forget_plain();
            }
            _ => self.clobber_all_memory(),
        }
    }

    /// Forgets every memory value except the runtime registers, which no
    /// pointer can address.
    fn clobber_all_memory(&mut self) {
        self.quad = None;
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
    returns: Returns,
    plain_memory: &HashSet<String>,
) -> bool {
    let plain = std::rc::Rc::new(plain_memory.clone());
    let live = live_after(insts, returns);
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
    known.plain = plain.clone();
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
                    known.plain = plain.clone();
                }
                continue;
            }
            // Inline assembly: afterwards nothing is known but Z = 0.
            Inst::Raw(_) => {
                let next_value = known.next_value;
                known = Known::entry();
                known.next_value = next_value;
                known.plain = plain.clone();
                for reg in [REG_A, REG_X, REG_Y] {
                    known.forget(reg);
                }
                continue;
            }
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
                        known.quad = None;
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
                if let (Some(key), Some(byte)) = (key.as_ref(), known.byte_in(reg)) {
                    known.values.push((key.clone(), format!("${byte:02x}")));
                }
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
                known.quad = None;
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
            // Logic on known bytes is a load of the answer; with A an
            // identity for the operation, a load of the operand (the same
            // read of the same memory).
            Op::And | Op::Ora | Op::Eor => {
                let a = known.byte_in(REG_A);
                if let (Some(a), Some(b)) = (a, known.byte_of(operand)) {
                    let result = match op {
                        Op::And => a & b,
                        Op::Ora => a | b,
                        _ => a ^ b,
                    };
                    let text = format!("${result:02x}");
                    rewrite.push((i, Inst::op(Op::Lda, Operand::Immediate(text.clone()))));
                    known.set(REG_A, Some(format!("#{text}")));
                    known.nz_from_a = true;
                    continue;
                }
                if matches!((op, a), (Op::Ora | Op::Eor, Some(0)) | (Op::And, Some(0xff))) {
                    rewrite.push((i, Inst::op(Op::Lda, operand.clone())));
                    let key = known.key(operand);
                    known.set(REG_A, key);
                    known.nz_from_a = true;
                    continue;
                }
                if let Some(constant) = known
                    .key(operand)
                    .filter(|key| !key.starts_with('#'))
                    .and_then(|key| known.constant(&key))
                {
                    rewrite.push((i, Inst::op(op, Operand::Immediate(constant))));
                }
                known.forget(REG_A);
                known.nz_from_a = true;
            }
            // An add of known bytes and a known carry is a load of the sum
            // (when the carry out is the carry in, or is not needed); with A
            // 0 and no carry in, a load of the operand (nothing carries out);
            // of 0 with no carry in, nothing.
            Op::Adc
                if known.carry.is_some() && {
                    let (a, b, carry_in) = (
                        known.byte_in(REG_A),
                        known.byte_of(operand),
                        known.carry == Some(true),
                    );
                    match (a, b) {
                        (Some(a), Some(b)) => {
                            let sum = u16::from(a) + u16::from(b) + u16::from(carry_in);
                            (sum > 0xff) == carry_in || carry_dead
                        }
                        (Some(0), None) => !carry_in,
                        (_, Some(0)) => !carry_in && (known.nz_from_a || flags_dead),
                        _ => false,
                    }
                } =>
            {
                let (a, b) = (known.byte_in(REG_A), known.byte_of(operand));
                let carry_in = known.carry == Some(true);
                match (a, b) {
                    (Some(a), Some(b)) => {
                        let sum = u16::from(a) + u16::from(b) + u16::from(carry_in);
                        let text = format!("${:02x}", sum as u8);
                        rewrite.push((i, Inst::op(Op::Lda, Operand::Immediate(text.clone()))));
                        known.set(REG_A, Some(format!("#{text}")));
                        known.nz_from_a = true;
                    }
                    (Some(0), None) => {
                        rewrite.push((i, Inst::op(Op::Lda, operand.clone())));
                        let key = known.key(operand);
                        known.set(REG_A, key);
                        known.nz_from_a = true;
                    }
                    _ => delete[i] = true,
                }
            }
            Op::Adc | Op::Sbc | Op::Pla => {
                if op != Op::Pla {
                    known.carry = None;
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
                if matches!(op, Op::Asl | Op::Lsr | Op::Rol | Op::Ror) {
                    known.carry = None;
                }
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
            Op::Cmp | Op::Cpx => {
                known.nz_from_a = false;
                known.carry = None;
            }
            Op::Jsr if matches!(operand, Operand::Label(name) if CLEAR_ROUTINES.contains(&name.as_str())) => {
                known.clobber_all_memory();
            }
            Op::Jsr => {
                let next_value = known.next_value;
                known = Known::entry();
                known.next_value = next_value;
                known.plain = plain.clone();
                for reg in [REG_A, REG_X, REG_Y] {
                    known.forget(reg);
                }
                known.nz_from_a =
                    matches!(operand, Operand::Label(name) if flag_routines.contains(name));
            }
            Op::Clc => known.carry = Some(false),
            Op::Sec => known.carry = Some(true),
            Op::Pha | Op::Phx | Op::Phy | Op::Phz | Op::Php | Op::Sei => {}
            Op::Plp => {
                known.nz_from_a = false;
                known.carry = None;
            }
            Op::Bit => known.nz_from_a = false,
            Op::Ldq | Op::Adcq => {
                let key = known.key(operand);
                // The registers already hold these four bytes.
                if op == Op::Ldq && flags_dead && key.is_some() && known.quad == key {
                    delete[i] = true;
                    continue;
                }
                for reg in [REG_A, REG_X, REG_Y, REG_Z] {
                    known.forget(reg);
                }
                known.nz_from_a = false;
                if op == Op::Adcq {
                    known.carry = None;
                } else {
                    known.quad = key;
                }
            }
            Op::Stq => {
                known.clobber(operand, 4);
                known.quad = known.key(operand);
            }
            Op::Bne
            | Op::Beq
            | Op::Bcc
            | Op::Bcs
            | Op::Bmi
            | Op::Bpl
            | Op::Bvs
            | Op::Bvc
            | Op::Lbne
            | Op::Lbeq
            | Op::Lbcc
            | Op::Lbcs
            | Op::Lbmi
            | Op::Lbpl
            | Op::Lbvs
            | Op::Lbvc => {}
            Op::Bra | Op::Lbra | Op::Rts | Op::Rtn | Op::Rti => {
                known = Known::default();
                known.plain = plain.clone();
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
fn carry_increments(
    insts: &mut Vec<Inst>,
    returns: Returns,
    plain_memory: &HashSet<String>,
) -> bool {
    let live = live_after(insts, returns);
    let mut i = 0;
    let mut changed = false;
    while i + 2 < insts.len() {
        // A run of `lda m / adc #0 / sta m` (the upper bytes of an add,
        // in place) is a ripple: `bcc + / inc m1 / bne + / inc m2 ...`,
        // since INC leaves Z set exactly when the byte wraps to 0.
        let chain: Vec<Operand> = (0..)
            .map(|k| i + 3 * k)
            .take_while(|&j| j + 2 < insts.len())
            .map_while(|j| match (&insts[j], &insts[j + 1], &insts[j + 2]) {
                (
                    Inst::Op(Op::Lda, load),
                    Inst::Op(Op::Adc, Operand::Immediate(zero)),
                    Inst::Op(Op::Sta, store),
                ) if zero == "$00"
                    && load == store
                    && matches!(load, Operand::Base(symbol) | Operand::Absolute(symbol)
                        if is_compiler_symbol(symbol) || plain_memory.contains(split_offset(symbol).0)) =>
                {
                    Some(load.clone())
                }
                _ => None,
            })
            .collect();
        if chain.len() >= 2 && live[i + 3 * chain.len() - 1] & (A | NZ | C) == 0 {
            let label = format!(
                "@m65_carry_{}",
                NEXT_CARRY_LABEL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let mut ripple = vec![Inst::op(Op::Bcc, Operand::Label(label.clone()))];
            for (k, byte) in chain.iter().enumerate() {
                if k > 0 {
                    ripple.push(Inst::op(Op::Bne, Operand::Label(label.clone())));
                }
                ripple.push(Inst::op(Op::Inc, byte.clone()));
            }
            ripple.push(Inst::Label(label));
            insts.splice(i..i + 3 * chain.len(), ripple);
            changed = true;
            i += 1;
            continue;
        }
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
                Inst::Raw(text) => i64::from(raw_size_bound(text)),
                _ => i64::from(inst.size().expect("only raw assembly has no size")),
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
    fn back_to_back_interrupt_holds_merge_unless_the_flags_are_read() {
        let returns = Returns {
            carry: false,
            bytes: Some(0),
        };
        let held = |tail: Vec<Inst>| {
            let mut insts = vec![
                Inst::implied(Op::Php),
                Inst::implied(Op::Sei),
                op(Op::Sta, Operand::Absolute("$d770".to_owned())),
                Inst::implied(Op::Plp),
                Inst::implied(Op::Php),
                Inst::implied(Op::Sei),
            ];
            insts.extend(tail);
            insts.extend([Inst::implied(Op::Plp), Inst::implied(Op::Rts)]);
            insts
        };
        let mut merged = held(vec![op(Op::Lda, Operand::imm_byte(1))]);
        assert!(merge_interrupt_holds(&mut merged, returns));
        assert_eq!(merged.len(), 6, "{merged:?}");
        // The carry `plp` restores is added in: the stretches stay apart.
        let mut apart = held(vec![op(Op::Adc, Operand::imm_byte(1))]);
        assert!(!merge_interrupt_holds(&mut apart, returns));
    }

    #[test]
    fn overwritten_stores_to_variables_go() {
        let plain: HashSet<String> = ["count".to_owned()].into_iter().collect();
        let count = Operand::Absolute("count".to_owned());
        let run = |between: Vec<Inst>| {
            let mut insts = vec![op(Op::Stz, count.clone())];
            insts.extend(between);
            insts.push(op(Op::Lda, Operand::Absolute("user".to_owned())));
            insts.push(op(Op::Sta, count.clone()));
            insts.push(Inst::implied(Op::Rts));
            optimize_routine_with(insts, true, &HashSet::new(), false, Some(0), &plain)
        };
        let first_store = op(Op::Stz, count.clone());
        // Stored again with nothing reading it between: the first goes.
        assert!(!run(vec![]).contains(&first_store));
        // A call might read it.
        assert!(run(vec![op(Op::Jsr, label("f"))]).contains(&first_store));
        // So might a read through a pointer.
        let pointer = vec![op(Op::Lda, Operand::FlatZ("__m65_farp".to_owned())), op(Op::Sta, Operand::Absolute("out".to_owned()))];
        assert!(run(pointer).contains(&first_store));
    }

    #[test]
    fn loop_counters_live_in_x() {
        let counter = Operand::Absolute("__static_f_i".to_owned());
        let looped = |body: Vec<Inst>| {
            let mut insts = vec![
                op(Op::Stz, counter.clone()),
                Inst::Label("top".to_owned()),
                op(Op::Lda, counter.clone()),
                op(Op::Cmp, imm("$0a")),
                op(Op::Lbeq, label("done")),
            ];
            insts.extend(body);
            insts.push(op(Op::Inc, counter.clone()));
            insts.push(op(Op::Lbra, label("top")));
            insts.push(Inst::Label("done".to_owned()));
            insts.push(Inst::implied(Op::Rts));
            optimize_routine_with(insts, true, &HashSet::new(), false, Some(0), &HashSet::new())
        };
        let store = op(Op::Sta, Operand::Absolute("out".to_owned()));
        let out = looped(vec![op(Op::Lda, imm("$01")), store.clone()]);
        assert!(!out.iter().any(|inst| matches!(inst, Inst::Op(_, operand) if *operand == counter)), "{out:?}");
        assert!(out.contains(&Inst::implied(Op::Inx)), "{out:?}");
        assert!(out.contains(&op(Op::Cpx, imm("$0a"))), "{out:?}");
        // A call in the loop may change X: the counter stays in memory.
        let out = looped(vec![op(Op::Jsr, label("f"))]);
        assert!(out.contains(&op(Op::Inc, counter.clone())), "{out:?}");
    }

    #[test]
    fn ordinary_variables_change_only_where_written() {
        let plain: HashSet<String> = ["count".to_owned()].into_iter().collect();
        let count = Operand::Absolute("count".to_owned());
        let port = Operand::Absolute("port".to_owned());
        let run = |between: Vec<Inst>, variable: &Operand| {
            let mut insts = vec![
                op(Op::Lda, Operand::Absolute("user".to_owned())),
                op(Op::Sta, variable.clone()),
            ];
            insts.extend(between);
            insts.push(op(Op::Lda, variable.clone()));
            insts.push(op(Op::Sta, Operand::Absolute("out".to_owned())));
            insts.push(Inst::implied(Op::Rts));
            optimize_routine_with(insts, true, &HashSet::new(), false, Some(0), &plain)
        };
        let reloads = |out: &[Inst], variable: &Operand| {
            out.iter()
                .filter(|inst| **inst == op(Op::Lda, variable.clone()))
                .count()
        };
        // Stored then read with nothing between: A still holds it.
        assert_eq!(reloads(&run(vec![], &count), &count), 0);
        // Not one of the ordinary variables (fixed, volatile, elsewhere).
        assert_eq!(reloads(&run(vec![], &port), &port), 1);
        // A write through a pointer may reach it.
        let pointer = vec![
            op(Op::Ldx, imm("$00")),
            op(Op::Stx, Operand::FlatZ("__m65_farp".to_owned())),
        ];
        assert_eq!(reloads(&run(pointer, &count), &count), 1);
        // A store to hardware (DMA, say) may change memory.
        let hardware = vec![op(Op::Stx, port.clone())];
        assert_eq!(reloads(&run(hardware, &count), &count), 1);
    }

    #[test]
    fn inline_assembly_is_an_opaque_step() {
        let scratch = base("__dp_scratch_0");
        let insts = vec![
            // Optimised around the block: the reload of A goes.
            op(Op::Lda, imm("$05")),
            op(Op::Sta, scratch.clone()),
            op(Op::Lda, scratch.clone()),
            // Kept: the block may read the scratch byte.
            op(Op::Sta, Operand::Absolute("out".to_owned())),
            Inst::Raw("\t.scope\n\tlda __dp_scratch_0\n\t.endscope".to_owned()),
            // After it nothing is known: this load stays.
            op(Op::Lda, scratch.clone()),
            op(Op::Sta, Operand::Absolute("out".to_owned())),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert_eq!(
            out.iter().filter(|inst| **inst == op(Op::Lda, scratch.clone())).count(),
            1,
            "{out:?}"
        );
        assert!(out.contains(&op(Op::Sta, scratch)), "{out:?}");
    }

    #[test]
    fn inline_assembly_naming_a_label_leaves_the_routine_alone() {
        let insts = vec![
            op(Op::Lda, imm("$05")),
            op(Op::Lda, imm("$05")),
            Inst::Label("back".to_owned()),
            Inst::Raw("\tjmp back".to_owned()),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts.clone(), true);
        assert_eq!(out, insts);
    }

    #[test]
    fn reloads_of_what_the_quad_registers_hold_go() {
        let total = Operand::Absolute("__static_f_total".to_owned());
        let insts = vec![
            Inst::implied(Op::Clc),
            op(Op::Ldq, Operand::Absolute("__static_f_first".to_owned())),
            op(Op::Adcq, Operand::Absolute("__static_f_second".to_owned())),
            op(Op::Stq, total.clone()),
            op(Op::Ldz, imm("$00")),
            op(Op::Ldq, total.clone()),
            op(Op::Stq, Operand::Absolute("out".to_owned())),
            op(Op::Ldz, imm("$00")),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        // The LDZ between them is dead (the reload sets Z again), so the
        // registers still hold `total` and the reload goes.
        assert!(!out.contains(&op(Op::Ldq, total)), "{out:?}");
        // A store between them would keep it: Z changes.
        let insts = vec![
            op(Op::Ldq, Operand::Absolute("__static_f_first".to_owned())),
            op(Op::Stq, Operand::Absolute("__static_f_total".to_owned())),
            op(Op::Ldz, imm("$00")),
            op(Op::Stz, Operand::Absolute("other".to_owned())),
            op(Op::Ldq, Operand::Absolute("__static_f_total".to_owned())),
            op(Op::Stq, Operand::Absolute("out".to_owned())),
            op(Op::Ldz, imm("$00")),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert!(out.contains(&op(Op::Ldq, Operand::Absolute("__static_f_total".to_owned()))), "{out:?}");
    }

    #[test]
    fn carry_chains_in_place_ripple() {
        // A parameter: a local nothing reads afterwards would just go.
        let byte = |k: u32| {
            if k == 0 {
                Operand::Absolute("__dp_f_param_at".to_owned())
            } else {
                Operand::Absolute(format!("__dp_f_param_at+{k}"))
            }
        };
        let mut insts = vec![
            Inst::implied(Op::Clc),
            op(Op::Lda, byte(0)),
            op(Op::Adc, imm("$10")),
            op(Op::Sta, byte(0)),
        ];
        for k in 1..4 {
            insts.push(op(Op::Lda, byte(k)));
            insts.push(op(Op::Adc, imm("$00")));
            insts.push(op(Op::Sta, byte(k)));
        }
        // Then on to other work: A, N/Z and C are given new values.
        insts.push(Inst::implied(Op::Clc));
        insts.push(op(Op::Lda, imm("$01")));
        insts.push(op(Op::Sta, Operand::Absolute("out".to_owned())));
        insts.push(Inst::implied(Op::Rts));
        let out = optimize_routine(insts, true);
        let incs = out.iter().filter(|inst| matches!(inst, Inst::Op(Op::Inc, _))).count();
        let adcs = out.iter().filter(|inst| matches!(inst, Inst::Op(Op::Adc, _))).count();
        assert_eq!((incs, adcs), (3, 1), "{out:?}");
    }

    #[test]
    fn dead_stores_to_temporaries_go() {
        let insts = vec![
            // Overwritten before it is read.
            op(Op::Lda, Operand::Absolute("user".to_owned())),
            op(Op::Sta, base("__dp_scratch_0")),
            op(Op::Lda, Operand::Absolute("other".to_owned())),
            op(Op::Sta, base("__dp_scratch_0")),
            op(Op::Ldx, base("__dp_scratch_0")),
            op(Op::Stx, Operand::Absolute("out".to_owned())),
            // Read by nothing before the return.
            op(Op::Sta, base("__dp_f_temp_t")),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        let stores = |symbol: &str| {
            out.iter()
                .filter(|inst| **inst == op(Op::Sta, base(symbol)))
                .count()
        };
        assert_eq!(stores("__dp_scratch_0"), 0, "{out:?}");
        assert_eq!(stores("__dp_f_temp_t"), 0, "{out:?}");
    }

    #[test]
    fn overwritten_stores_to_locals_go() {
        let texel = Operand::Absolute("__static_f_texel".to_owned());
        let insts = vec![
            op(Op::Lda, Operand::Absolute("user".to_owned())),
            op(Op::Stz, texel.clone()),
            op(Op::Sta, texel.clone()),
            op(Op::Lda, texel.clone()),
            op(Op::Sta, Operand::Absolute("out".to_owned())),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert!(!out.contains(&op(Op::Stz, texel)), "{out:?}");
    }

    #[test]
    fn stores_to_temporaries_before_a_call_stay() {
        let insts = vec![
            op(Op::Lda, Operand::Absolute("user".to_owned())),
            op(Op::Sta, base("__dp_scratch_1")),
            op(Op::Jsr, label("f")),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert!(out.contains(&op(Op::Sta, base("__dp_scratch_1"))), "{out:?}");
    }

    #[test]
    fn adds_of_known_bytes_fold() {
        let insts = vec![
            Inst::implied(Op::Clc),
            op(Op::Lda, imm("$00")),
            op(Op::Adc, imm("$00")),
            op(Op::Sta, base("__dp_scratch_0")),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert!(!out.iter().any(|inst| matches!(inst, Inst::Op(Op::Adc, _))), "{out:?}");
    }

    #[test]
    fn logic_with_an_identity_loads_the_operand() {
        let user = Operand::Absolute("user".to_owned());
        let insts = vec![
            op(Op::Lda, imm("$00")),
            op(Op::Ora, user.clone()),
            op(Op::Sta, Operand::Absolute("out".to_owned())),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert!(out.contains(&op(Op::Lda, user)), "{out:?}");
        assert!(!out.iter().any(|inst| matches!(inst, Inst::Op(Op::Ora, _))), "{out:?}");
    }

    #[test]
    fn stored_constants_outlive_the_register() {
        let insts = vec![
            op(Op::Lda, imm("$05")),
            op(Op::Sta, base("__dp_scratch_0")),
            op(Op::Lda, Operand::Absolute("user".to_owned())),
            op(Op::Sta, base("__dp_scratch_1")),
            op(Op::Lda, base("__dp_scratch_0")),
            op(Op::Sta, Operand::Absolute("out".to_owned())),
            Inst::implied(Op::Rts),
        ];
        let out = optimize_routine(insts, true);
        assert!(!out.contains(&op(Op::Lda, base("__dp_scratch_0"))), "{out:?}");
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
        let out = optimize_routine_with(insts, true, &flag_routines, false, None, &HashSet::new());
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
