use std::collections::{BTreeMap, BTreeSet};

use crate::target_65816::{AsmInst, AsmModule, AsmRoutine, Immediate, Mode, Operand, Symbol};

const SHORT_BRANCH_DISTANCE: usize = 40;

pub fn optimize_module(module: &AsmModule) -> AsmModule {
    let direct_symbols = module
        .direct_symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect::<BTreeSet<_>>();

    AsmModule {
        name: module.name.clone(),
        direct_symbols: module.direct_symbols.clone(),
        static_symbols: module.static_symbols.clone(),
        static_data: module.static_data.clone(),
        fixed_symbols: module.fixed_symbols.clone(),
        banked_symbols: module.banked_symbols.clone(),
        vectors: module.vectors.clone(),
        metadata: module.metadata.clone(),
        imports: module.imports.clone(),
        libraries: module.libraries.clone(),
        external_routines: module.external_routines.clone(),
        external_direct_symbols: module.external_direct_symbols.clone(),
        entry: module.entry.clone(),
        routines: module
            .routines
            .iter()
            .map(|routine| optimize_routine(routine, &direct_symbols))
            .collect(),
    }
}

fn optimize_routine(routine: &AsmRoutine, direct_symbols: &BTreeSet<String>) -> AsmRoutine {
    let insts = prefer_direct_operands(&routine.insts, direct_symbols);
    let insts = shorten_branches(&insts);
    let insts = use_relative_pushes(&insts);
    let insts = peephole(&insts);
    let insts = remove_redundant_modes(&insts);
    let insts = remove_redundant_accumulator_loads(&insts);
    let insts = remove_redundant_index_loads(&insts);
    let insts = remove_redundant_index_transfers(&insts);
    let insts = remove_redundant_modes(&insts);
    let insts = remove_redundant_zero_compares(&insts);

    AsmRoutine {
        name: routine.name.clone(),
        export: routine.export,
        insts,
    }
}

/// Drops `cmp #0` when the previous instruction already set N and Z from A at
/// the same width, and only an equality branch consumes the flags (the
/// compare's carry result is never read).
fn remove_redundant_zero_compares(insts: &[AsmInst]) -> Vec<AsmInst> {
    let mut out = Vec::with_capacity(insts.len());
    for (idx, inst) in insts.iter().enumerate() {
        let redundant = matches!(
            inst,
            AsmInst::Cmp(Operand::Immediate(Immediate::Byte(0) | Immediate::Word(0)))
        ) && idx > 0
            && sets_zero_flag_from_a(&insts[idx - 1])
            && insts.get(idx + 1).is_some_and(is_equality_branch)
            && !insts.get(idx + 2).is_some_and(reads_carry);
        if !redundant {
            out.push(inst.clone());
        }
    }
    out
}

fn sets_zero_flag_from_a(inst: &AsmInst) -> bool {
    matches!(
        inst,
        AsmInst::Lda(_)
            | AsmInst::And(_)
            | AsmInst::Ora(_)
            | AsmInst::Adc(_)
            | AsmInst::Sbc(_)
            | AsmInst::Txa
            | AsmInst::Tya
            | AsmInst::Pla
            | AsmInst::AslA
            | AsmInst::LsrA
    )
}

fn is_equality_branch(inst: &AsmInst) -> bool {
    match inst {
        AsmInst::Beq(_) | AsmInst::Bne(_) => true,
        // Long equality branches are emitted as `bne :+ / brl target / :`.
        AsmInst::Raw(text) => text.starts_with("bne :+\n") || text.starts_with("beq :+\n"),
        _ => false,
    }
}

fn reads_carry(inst: &AsmInst) -> bool {
    match inst {
        AsmInst::Bcc(_) | AsmInst::Bcs(_) | AsmInst::Adc(_) | AsmInst::Sbc(_) => true,
        AsmInst::Raw(text) => text.starts_with("bcc") || text.starts_with("bcs"),
        _ => false,
    }
}

fn prefer_direct_operands(insts: &[AsmInst], direct_symbols: &BTreeSet<String>) -> Vec<AsmInst> {
    insts
        .iter()
        .map(|inst| {
            map_operands(inst, &mut |operand| {
                prefer_direct_operand(operand, direct_symbols)
            })
        })
        .collect()
}

fn prefer_direct_operand(operand: &Operand, direct_symbols: &BTreeSet<String>) -> Operand {
    match operand {
        Operand::Absolute(Symbol::Named(name)) if direct_symbols.contains(name) => {
            Operand::Direct(Symbol::Named(name.clone()))
        }
        Operand::AbsoluteIndexedX(Symbol::Named(name)) if direct_symbols.contains(name) => {
            Operand::DirectIndexedX(Symbol::Named(name.clone()))
        }
        other => other.clone(),
    }
}

fn map_operands(inst: &AsmInst, map: &mut impl FnMut(&Operand) -> Operand) -> AsmInst {
    match inst {
        AsmInst::Lda(operand) => AsmInst::Lda(map(operand)),
        AsmInst::Ldx(operand) => AsmInst::Ldx(map(operand)),
        AsmInst::Ldy(operand) => AsmInst::Ldy(map(operand)),
        AsmInst::Sta(operand) => AsmInst::Sta(map(operand)),
        AsmInst::Stx(operand) => AsmInst::Stx(map(operand)),
        AsmInst::Sty(operand) => AsmInst::Sty(map(operand)),
        AsmInst::Stz(operand) => AsmInst::Stz(map(operand)),
        AsmInst::Adc(operand) => AsmInst::Adc(map(operand)),
        AsmInst::Sbc(operand) => AsmInst::Sbc(map(operand)),
        AsmInst::And(operand) => AsmInst::And(map(operand)),
        AsmInst::Ora(operand) => AsmInst::Ora(map(operand)),
        AsmInst::Inc(operand) => AsmInst::Inc(map(operand)),
        AsmInst::Dec(operand) => AsmInst::Dec(map(operand)),
        AsmInst::Cmp(operand) => AsmInst::Cmp(map(operand)),
        AsmInst::Cpx(operand) => AsmInst::Cpx(map(operand)),
        other => other.clone(),
    }
}

fn shorten_branches(insts: &[AsmInst]) -> Vec<AsmInst> {
    let labels = insts
        .iter()
        .enumerate()
        .filter_map(|(idx, inst)| match inst {
            AsmInst::Label(label) => Some((label.clone(), idx)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();

    insts
        .iter()
        .enumerate()
        .map(|(idx, inst)| match inst {
            AsmInst::Brl(label) if branch_is_short(idx, label, &labels) => {
                AsmInst::Bra(label.clone())
            }
            other => other.clone(),
        })
        .collect()
}

fn use_relative_pushes(insts: &[AsmInst]) -> Vec<AsmInst> {
    let labels = insts
        .iter()
        .filter_map(|inst| match inst {
            AsmInst::Label(label) => Some(label.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    insts
        .iter()
        .map(|inst| match inst {
            AsmInst::Pea(Symbol::Named(label)) if labels.contains(label) => {
                AsmInst::Per(label.clone())
            }
            other => other.clone(),
        })
        .collect()
}

fn branch_is_short(idx: usize, label: &str, labels: &BTreeMap<String, usize>) -> bool {
    labels
        .get(label)
        .is_some_and(|target| idx.abs_diff(*target) <= SHORT_BRANCH_DISTANCE)
}

fn peephole(insts: &[AsmInst]) -> Vec<AsmInst> {
    let mut out = Vec::with_capacity(insts.len());
    let mut i = 0;

    while i < insts.len() {
        if let Some((replacement, consumed)) = peephole_at(&insts[i..]) {
            out.extend(replacement);
            i += consumed;
        } else {
            out.push(insts[i].clone());
            i += 1;
        }
    }

    out
}

fn peephole_at(insts: &[AsmInst]) -> Option<(Vec<AsmInst>, usize)> {
    if let [AsmInst::Mode(Mode::Acc8Index16 | Mode::Acc8Index8), AsmInst::Lda(Operand::Immediate(Immediate::Byte(hi))), AsmInst::Xba, AsmInst::Lda(Operand::Immediate(Immediate::Byte(lo))), AsmInst::Mode(final_mode @ (Mode::Acc16Index16 | Mode::Acc16Index8)), ..] =
        insts
    {
        if xba_compose_preserves_flags(*hi, *lo) {
            return Some((
                vec![
                    AsmInst::Mode(*final_mode),
                    AsmInst::Lda(Operand::Immediate(Immediate::Word(
                        ((*hi as u16) << 8) | *lo as u16,
                    ))),
                ],
                5,
            ));
        }
    }

    if let [AsmInst::Mode(mode @ (Mode::Acc16Index16 | Mode::Acc16Index8)), AsmInst::Lda(Operand::Immediate(Immediate::Word(value))), AsmInst::Pha, AsmInst::Lda(Operand::Immediate(Immediate::Word(reloaded))), ..] =
        insts
    {
        if value == reloaded {
            return Some((
                vec![
                    AsmInst::Pea(Symbol::Address(*value as u32)),
                    AsmInst::Mode(*mode),
                    AsmInst::Lda(Operand::Immediate(Immediate::Word(*value))),
                ],
                4,
            ));
        }
    }

    if let [AsmInst::Mode(mode), AsmInst::Lda(Operand::Immediate(zero)), AsmInst::Sta(target), ..] =
        insts
    {
        if immediate_is_zero(*zero) && stz_operand(target) {
            return Some((vec![AsmInst::Mode(*mode), AsmInst::Stz(target.clone())], 3));
        }
    }

    if let [AsmInst::Mode(mode), AsmInst::Lda(load), AsmInst::Clc, AsmInst::Adc(Operand::Immediate(one)), AsmInst::Sta(store), ..] =
        insts
    {
        if load == store && immediate_is_one(*one) && inc_dec_operand(store) {
            return Some((vec![AsmInst::Mode(*mode), AsmInst::Inc(store.clone())], 5));
        }
    }

    if let [AsmInst::Mode(mode), AsmInst::Lda(load), AsmInst::Sec, AsmInst::Sbc(Operand::Immediate(one)), AsmInst::Sta(store), ..] =
        insts
    {
        if load == store && immediate_is_one(*one) && inc_dec_operand(store) {
            return Some((vec![AsmInst::Mode(*mode), AsmInst::Dec(store.clone())], 5));
        }
    }

    None
}

fn xba_compose_preserves_flags(hi: u8, lo: u8) -> bool {
    let zero_flag_matches = lo != 0 || hi == 0;
    let negative_flag_matches = (hi & 0x80) == (lo & 0x80);

    zero_flag_matches && negative_flag_matches
}

fn immediate_is_zero(value: Immediate) -> bool {
    matches!(
        value,
        Immediate::Byte(0) | Immediate::Word(0) | Immediate::Long(0)
    )
}

fn immediate_is_one(value: Immediate) -> bool {
    matches!(
        value,
        Immediate::Byte(1) | Immediate::Word(1) | Immediate::Long(1)
    )
}

fn stz_operand(operand: &Operand) -> bool {
    matches!(
        operand,
        Operand::Absolute(Symbol::Address(0..=0xffff))
            | Operand::AbsoluteIndexedX(Symbol::Address(0..=0xffff))
            | Operand::Direct(_)
            | Operand::DirectIndexedX(_)
    )
}

fn inc_dec_operand(operand: &Operand) -> bool {
    matches!(
        operand,
        Operand::Absolute(Symbol::Address(0..=0xffff))
            | Operand::AbsoluteIndexedX(Symbol::Address(0..=0xffff))
            | Operand::Direct(_)
            | Operand::DirectIndexedX(_)
    )
}

fn remove_redundant_accumulator_loads(insts: &[AsmInst]) -> Vec<AsmInst> {
    let mut out = Vec::with_capacity(insts.len());
    let mut state = AccumulatorState::default();

    for inst in insts {
        match inst {
            AsmInst::Lda(operand)
                if state.flags_match_a
                    && state.aliases.iter().any(|alias| alias == operand)
                    && accumulator_load_cacheable(operand) => {}
            AsmInst::Lda(operand) => {
                out.push(inst.clone());
                state.load(operand);
            }
            AsmInst::Sta(operand)
                if state.aliases.iter().any(|alias| alias == operand)
                    && accumulator_store_cacheable(operand) => {}
            AsmInst::Sta(operand) => {
                out.push(inst.clone());
                state.store_a_to(operand);
            }
            AsmInst::Adc(_)
            | AsmInst::Sbc(_)
            | AsmInst::And(_)
            | AsmInst::Ora(_)
            | AsmInst::AslA
            | AsmInst::LsrA => {
                out.push(inst.clone());
                state.a_changed_with_matching_flags();
            }
            AsmInst::Txa | AsmInst::Tya => {
                out.push(inst.clone());
                state.a_changed_with_matching_flags();
            }
            AsmInst::Cmp(_)
            | AsmInst::Cpx(_)
            | AsmInst::Tax
            | AsmInst::Tay
            | AsmInst::Ldx(_)
            | AsmInst::Ldy(_)
            | AsmInst::Inx
            | AsmInst::Inc(_)
            | AsmInst::Dec(_) => {
                out.push(inst.clone());
                state.flags_no_longer_match_a();
            }
            AsmInst::Stx(operand) | AsmInst::Sty(operand) | AsmInst::Stz(operand) => {
                out.push(inst.clone());
                state.invalidate_memory_aliases_for_store(operand);
            }
            AsmInst::Mode(_)
            | AsmInst::Label(_)
            | AsmInst::Jsr(_)
            | AsmInst::Jsl(_)
            | AsmInst::Rts
            | AsmInst::Rtl
            | AsmInst::Rti
            | AsmInst::Raw(_)
            | AsmInst::Xba
            | AsmInst::Tsc
            | AsmInst::Tcs => {
                out.push(inst.clone());
                state.clear();
            }
            AsmInst::Pha
            | AsmInst::Phx
            | AsmInst::Phy
            | AsmInst::Php
            | AsmInst::Phb
            | AsmInst::Phk
            | AsmInst::Pea(_)
            | AsmInst::Per(_) => {
                out.push(inst.clone());
                state.invalidate_stack_aliases();
            }
            AsmInst::Pla => {
                out.push(inst.clone());
                state.invalidate_stack_aliases();
                state.a_changed_with_matching_flags();
            }
            AsmInst::Plx | AsmInst::Ply | AsmInst::Plp | AsmInst::Plb => {
                out.push(inst.clone());
                state.invalidate_stack_aliases();
                state.flags_no_longer_match_a();
            }
            AsmInst::Bra(_) | AsmInst::Brl(_) => {
                out.push(inst.clone());
                state.clear();
            }
            AsmInst::Clc
            | AsmInst::Sec
            | AsmInst::Beq(_)
            | AsmInst::Bne(_)
            | AsmInst::Bcc(_)
            | AsmInst::Bcs(_)
            | AsmInst::Nop
            | AsmInst::Stp => out.push(inst.clone()),
        }
    }

    out
}

#[derive(Debug, Default)]
struct AccumulatorState {
    aliases: Vec<Operand>,
    flags_match_a: bool,
}

impl AccumulatorState {
    fn clear(&mut self) {
        self.aliases.clear();
        self.flags_match_a = false;
    }

    fn load(&mut self, operand: &Operand) {
        self.aliases.clear();
        if accumulator_load_cacheable(operand) {
            self.aliases.push(operand.clone());
        }
        self.flags_match_a = true;
    }

    fn store_a_to(&mut self, operand: &Operand) {
        self.invalidate_memory_aliases_for_store(operand);
        if self.flags_match_a && accumulator_store_cacheable(operand) {
            self.add_alias(operand.clone());
        }
    }

    fn a_changed_with_matching_flags(&mut self) {
        self.aliases.clear();
        self.flags_match_a = true;
    }

    fn flags_no_longer_match_a(&mut self) {
        self.flags_match_a = false;
    }

    fn invalidate_memory_aliases_for_store(&mut self, operand: &Operand) {
        self.aliases
            .retain(|alias| operand_alias_survives_store(alias, operand));
    }

    fn invalidate_stack_aliases(&mut self) {
        self.aliases.retain(operand_survives_stack_pointer_change);
    }

    fn add_alias(&mut self, operand: Operand) {
        if !self.aliases.iter().any(|alias| alias == &operand) {
            self.aliases.push(operand);
        }
    }
}

fn operand_survives_stack_pointer_change(operand: &Operand) -> bool {
    !matches!(
        operand,
        Operand::StackRelative(_) | Operand::StackRelativeIndirectIndexedY(_)
    )
}

fn operand_alias_survives_store(alias: &Operand, store: &Operand) -> bool {
    if alias == store {
        return true;
    }
    !matches!(
        (alias, store),
        (Operand::Absolute(_), Operand::Absolute(_))
            | (Operand::Absolute(_), Operand::AbsoluteIndexedX(_))
            | (Operand::AbsoluteIndexedX(_), Operand::Absolute(_))
            | (Operand::AbsoluteIndexedX(_), Operand::AbsoluteIndexedX(_))
            | (Operand::Direct(_), Operand::Direct(_))
            | (Operand::Direct(_), Operand::DirectIndexedX(_))
            | (Operand::DirectIndexedX(_), Operand::Direct(_))
            | (Operand::DirectIndexedX(_), Operand::DirectIndexedX(_))
            | (Operand::StackRelative(_), Operand::StackRelative(_))
            | (
                Operand::StackRelativeIndirectIndexedY(_),
                Operand::StackRelativeIndirectIndexedY(_)
            )
    )
}

fn memory_alias_survives_unrelated_store(alias: &Operand, store: &Operand) -> bool {
    match (alias, store) {
        (Operand::Immediate(_), _) | (Operand::ImmediateSymbol(_), _) => true,
        _ if alias == store => false,
        _ => operand_alias_survives_store(alias, store),
    }
}

fn accumulator_load_cacheable(operand: &Operand) -> bool {
    matches!(
        operand,
        Operand::Immediate(_)
            | Operand::ImmediateSymbol(_)
            | Operand::Direct(_)
            | Operand::DirectIndexedX(_)
            | Operand::StackRelative(_)
    )
}

fn accumulator_store_cacheable(operand: &Operand) -> bool {
    matches!(operand, Operand::Direct(_) | Operand::StackRelative(_))
}

fn remove_redundant_index_loads(insts: &[AsmInst]) -> Vec<AsmInst> {
    let mut out = Vec::with_capacity(insts.len());
    let mut state = IndexLoadState::default();

    for inst in insts {
        match inst {
            AsmInst::Ldx(operand)
                if state.x.flags_match
                    && state.x.aliases.iter().any(|alias| alias == operand)
                    && index_load_cacheable(operand) => {}
            AsmInst::Ldy(operand)
                if state.y.flags_match
                    && state.y.aliases.iter().any(|alias| alias == operand)
                    && index_load_cacheable(operand) => {}
            _ => {
                out.push(inst.clone());
                state.observe(inst);
            }
        }
    }

    out
}

#[derive(Debug, Default)]
struct IndexLoadState {
    x: RegisterAliasState,
    y: RegisterAliasState,
}

impl IndexLoadState {
    fn observe(&mut self, inst: &AsmInst) {
        match inst {
            AsmInst::Ldx(operand) => {
                self.x.load(operand, index_load_cacheable);
                self.y.flags_no_longer_match();
            }
            AsmInst::Ldy(operand) => {
                self.x.flags_no_longer_match();
                self.y.load(operand, index_load_cacheable);
            }
            AsmInst::Tax => {
                self.x.aliases.clear();
                self.x.flags_match = true;
                self.y.flags_no_longer_match();
            }
            AsmInst::Tay => {
                self.y.aliases.clear();
                self.y.flags_match = true;
                self.x.flags_no_longer_match();
            }
            AsmInst::Txa => {
                self.x.flags_match = true;
                self.y.flags_no_longer_match();
            }
            AsmInst::Tya => {
                self.x.flags_no_longer_match();
                self.y.flags_match = true;
            }
            AsmInst::Inx => {
                self.x.aliases.clear();
                self.x.flags_match = true;
                self.y.flags_no_longer_match();
            }
            AsmInst::Lda(_)
            | AsmInst::Adc(_)
            | AsmInst::Sbc(_)
            | AsmInst::And(_)
            | AsmInst::Ora(_)
            | AsmInst::AslA
            | AsmInst::LsrA => {
                self.x.flags_no_longer_match();
                self.y.flags_no_longer_match();
            }
            AsmInst::Cmp(_) | AsmInst::Cpx(_) | AsmInst::Inc(_) | AsmInst::Dec(_) => {
                self.x.flags_no_longer_match();
                self.y.flags_no_longer_match();
            }
            AsmInst::Stx(operand) => {
                self.x.invalidate_memory_aliases_for_store(operand);
                self.y
                    .invalidate_memory_aliases_for_unrelated_store(operand);
                if index_store_cacheable(operand) {
                    self.x.add_alias(operand.clone());
                }
            }
            AsmInst::Sty(operand) => {
                self.x
                    .invalidate_memory_aliases_for_unrelated_store(operand);
                self.y.invalidate_memory_aliases_for_store(operand);
                if index_store_cacheable(operand) {
                    self.y.add_alias(operand.clone());
                }
            }
            AsmInst::Sta(operand) | AsmInst::Stz(operand) => {
                self.x
                    .invalidate_memory_aliases_for_unrelated_store(operand);
                self.y
                    .invalidate_memory_aliases_for_unrelated_store(operand);
            }
            AsmInst::Mode(_)
            | AsmInst::Label(_)
            | AsmInst::Jsr(_)
            | AsmInst::Jsl(_)
            | AsmInst::Rts
            | AsmInst::Rtl
            | AsmInst::Rti
            | AsmInst::Raw(_)
            | AsmInst::Xba
            | AsmInst::Tsc
            | AsmInst::Tcs
            | AsmInst::Pla
            | AsmInst::Plx
            | AsmInst::Ply
            | AsmInst::Plp
            | AsmInst::Plb
            | AsmInst::Bra(_)
            | AsmInst::Brl(_)
            | AsmInst::Beq(_)
            | AsmInst::Bne(_)
            | AsmInst::Bcc(_)
            | AsmInst::Bcs(_) => self.clear(),
            AsmInst::Pha
            | AsmInst::Phx
            | AsmInst::Phy
            | AsmInst::Php
            | AsmInst::Phb
            | AsmInst::Phk
            | AsmInst::Pea(_)
            | AsmInst::Per(_) => {
                self.x.invalidate_stack_aliases();
                self.y.invalidate_stack_aliases();
            }
            AsmInst::Clc | AsmInst::Sec | AsmInst::Nop | AsmInst::Stp => {}
        }
    }

    fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
    }
}

#[derive(Debug, Default)]
struct RegisterAliasState {
    aliases: Vec<Operand>,
    flags_match: bool,
}

impl RegisterAliasState {
    fn clear(&mut self) {
        self.aliases.clear();
        self.flags_match = false;
    }

    fn load(&mut self, operand: &Operand, cacheable: impl Fn(&Operand) -> bool) {
        self.aliases.clear();
        if cacheable(operand) {
            self.aliases.push(operand.clone());
        }
        self.flags_match = true;
    }

    fn flags_no_longer_match(&mut self) {
        self.flags_match = false;
    }

    fn invalidate_memory_aliases_for_store(&mut self, operand: &Operand) {
        self.aliases
            .retain(|alias| operand_alias_survives_store(alias, operand));
    }

    fn invalidate_memory_aliases_for_unrelated_store(&mut self, operand: &Operand) {
        self.aliases
            .retain(|alias| memory_alias_survives_unrelated_store(alias, operand));
    }

    fn invalidate_stack_aliases(&mut self) {
        self.aliases.retain(operand_survives_stack_pointer_change);
    }

    fn add_alias(&mut self, operand: Operand) {
        if !self.aliases.iter().any(|alias| alias == &operand) {
            self.aliases.push(operand);
        }
    }
}

fn remove_redundant_index_transfers(insts: &[AsmInst]) -> Vec<AsmInst> {
    let mut out = Vec::with_capacity(insts.len());
    let mut state = IndexTransferState::default();
    let mut i = 0;

    while i < insts.len() {
        if let [AsmInst::Lda(operand), AsmInst::Tax, next, ..] = &insts[i..] {
            if state.x_aliases.iter().any(|alias| alias == operand)
                && accumulator_load_cacheable(operand)
                && load_uses_index_x(next)
            {
                i += 2;
                continue;
            }
        }

        let inst = &insts[i];
        out.push(inst.clone());
        state.observe(inst);
        i += 1;
    }

    out
}

#[derive(Debug, Default)]
struct IndexTransferState {
    a_aliases: Vec<Operand>,
    x_aliases: Vec<Operand>,
}

impl IndexTransferState {
    fn observe(&mut self, inst: &AsmInst) {
        match inst {
            AsmInst::Lda(operand) => {
                self.a_aliases.clear();
                if accumulator_load_cacheable(operand) {
                    self.a_aliases.push(operand.clone());
                }
            }
            AsmInst::Ldx(operand) => {
                self.x_aliases.clear();
                if index_load_cacheable(operand) {
                    self.x_aliases.push(operand.clone());
                }
            }
            AsmInst::Tax => {
                self.x_aliases = self.a_aliases.clone();
            }
            AsmInst::Txa => {
                self.a_aliases = self.x_aliases.clone();
            }
            AsmInst::Sta(operand) => {
                self.invalidate_a_memory_aliases_for_accumulator_store(operand);
                self.invalidate_x_memory_aliases_for_unrelated_store(operand);
            }
            AsmInst::Stx(operand) => {
                self.invalidate_a_memory_aliases_for_unrelated_store(operand);
                self.invalidate_x_memory_aliases_for_index_store(operand);
                if index_store_cacheable(operand) {
                    self.add_x_alias(operand.clone());
                }
            }
            AsmInst::Sty(operand) | AsmInst::Stz(operand) => {
                self.invalidate_a_memory_aliases_for_unrelated_store(operand);
                self.invalidate_x_memory_aliases_for_unrelated_store(operand);
            }
            AsmInst::Adc(_)
            | AsmInst::Sbc(_)
            | AsmInst::And(_)
            | AsmInst::Ora(_)
            | AsmInst::AslA
            | AsmInst::LsrA
            | AsmInst::Tya => {
                self.a_aliases.clear();
            }
            AsmInst::Inx => {
                self.x_aliases.clear();
            }
            AsmInst::Mode(_)
            | AsmInst::Label(_)
            | AsmInst::Jsr(_)
            | AsmInst::Jsl(_)
            | AsmInst::Rts
            | AsmInst::Rtl
            | AsmInst::Rti
            | AsmInst::Raw(_)
            | AsmInst::Xba
            | AsmInst::Tsc
            | AsmInst::Tcs
            | AsmInst::Pha
            | AsmInst::Phx
            | AsmInst::Phy
            | AsmInst::Php
            | AsmInst::Phb
            | AsmInst::Phk
            | AsmInst::Pea(_)
            | AsmInst::Per(_)
            | AsmInst::Pla
            | AsmInst::Plx
            | AsmInst::Ply
            | AsmInst::Plp
            | AsmInst::Plb
            | AsmInst::Bra(_)
            | AsmInst::Brl(_)
            | AsmInst::Beq(_)
            | AsmInst::Bne(_)
            | AsmInst::Bcc(_)
            | AsmInst::Bcs(_) => self.clear(),
            AsmInst::Ldy(_)
            | AsmInst::Tay
            | AsmInst::Cmp(_)
            | AsmInst::Cpx(_)
            | AsmInst::Inc(_)
            | AsmInst::Dec(_)
            | AsmInst::Clc
            | AsmInst::Sec
            | AsmInst::Nop
            | AsmInst::Stp => {}
        }
    }

    fn clear(&mut self) {
        self.a_aliases.clear();
        self.x_aliases.clear();
    }

    fn invalidate_a_memory_aliases_for_accumulator_store(&mut self, operand: &Operand) {
        self.a_aliases
            .retain(|alias| operand_alias_survives_store(alias, operand));
    }

    fn invalidate_a_memory_aliases_for_unrelated_store(&mut self, operand: &Operand) {
        self.a_aliases
            .retain(|alias| memory_alias_survives_unrelated_store(alias, operand));
    }

    fn invalidate_x_memory_aliases_for_index_store(&mut self, operand: &Operand) {
        self.x_aliases
            .retain(|alias| operand_alias_survives_store(alias, operand));
    }

    fn invalidate_x_memory_aliases_for_unrelated_store(&mut self, operand: &Operand) {
        self.x_aliases
            .retain(|alias| memory_alias_survives_unrelated_store(alias, operand));
    }

    fn add_x_alias(&mut self, operand: Operand) {
        if !self.x_aliases.iter().any(|alias| alias == &operand) {
            self.x_aliases.push(operand);
        }
    }
}

fn load_uses_index_x(inst: &AsmInst) -> bool {
    matches!(
        inst,
        AsmInst::Lda(Operand::AbsoluteIndexedX(_) | Operand::DirectIndexedX(_))
    )
}

fn index_load_cacheable(operand: &Operand) -> bool {
    matches!(
        operand,
        Operand::Immediate(_)
            | Operand::ImmediateSymbol(_)
            | Operand::Direct(_)
            | Operand::StackRelative(_)
    )
}

fn index_store_cacheable(operand: &Operand) -> bool {
    matches!(operand, Operand::Direct(_) | Operand::StackRelative(_))
}

fn remove_redundant_modes(insts: &[AsmInst]) -> Vec<AsmInst> {
    let mut out = Vec::with_capacity(insts.len());
    let mut mode = None;

    for inst in insts {
        match inst {
            AsmInst::Mode(next) if mode == Some(*next) => {}
            AsmInst::Mode(next) => {
                mode = Some(*next);
                out.push(inst.clone());
            }
            inst if resets_mode_state(inst) => {
                mode = None;
                out.push(inst.clone());
            }
            _ => out.push(inst.clone()),
        }
    }

    out
}

fn resets_mode_state(inst: &AsmInst) -> bool {
    matches!(
        inst,
        AsmInst::Label(_)
            | AsmInst::Jsr(_)
            | AsmInst::Jsl(_)
            | AsmInst::Rts
            | AsmInst::Rtl
            | AsmInst::Rti
            | AsmInst::Plp
            | AsmInst::Raw(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target_65816::DirectSymbol;

    fn routine(insts: Vec<AsmInst>) -> AsmRoutine {
        AsmRoutine::new("test").with_insts(insts)
    }

    fn module(insts: Vec<AsmInst>) -> AsmModule {
        AsmModule::new("test").with_routine(routine(insts))
    }

    fn optimized_insts(module: &AsmModule) -> Vec<AsmInst> {
        optimize_module(module).routines[0].insts.clone()
    }

    #[test]
    fn removes_redundant_modes_in_straight_line_code() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(1))),
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Sta(Operand::Absolute(Symbol::Address(0x2000))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Byte(1))),
                AsmInst::Sta(Operand::Absolute(Symbol::Address(0x2000))),
            ]
        );
    }

    #[test]
    fn preserves_mode_after_labels_for_branch_entry() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Label("again".to_owned()),
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(1))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Label("again".to_owned()),
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Byte(1))),
            ]
        );
    }

    #[test]
    fn replaces_load_zero_store_with_stz() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(0))),
            AsmInst::Sta(Operand::Absolute(Symbol::Address(0x2000))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Stz(Operand::Absolute(Symbol::Address(0x2000))),
            ]
        );
    }

    #[test]
    fn replaces_self_add_one_with_inc() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("counter".to_owned()))),
            AsmInst::Clc,
            AsmInst::Adc(Operand::Immediate(Immediate::Byte(1))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("counter".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Inc(Operand::Direct(Symbol::Named("counter".to_owned()))),
            ]
        );
    }

    #[test]
    fn replaces_self_sub_one_with_dec() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("counter".to_owned()))),
            AsmInst::Sec,
            AsmInst::Sbc(Operand::Immediate(Immediate::Byte(1))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("counter".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Dec(Operand::Direct(Symbol::Named("counter".to_owned()))),
            ]
        );
    }

    #[test]
    fn removes_redundant_direct_reload_after_store() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc16Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Lda(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Adc(Operand::Immediate(Immediate::Word(1))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc16Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
                AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
                AsmInst::Adc(Operand::Immediate(Immediate::Word(1))),
            ]
        );
    }

    #[test]
    fn removes_redundant_direct_reload_after_stack_push() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc16Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Pha,
            AsmInst::Lda(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Adc(Operand::Immediate(Immediate::Word(1))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc16Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
                AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
                AsmInst::Pha,
                AsmInst::Adc(Operand::Immediate(Immediate::Word(1))),
            ]
        );
    }

    #[test]
    fn keeps_stack_relative_reload_after_stack_push() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc16Index16),
            AsmInst::Lda(Operand::StackRelative(4)),
            AsmInst::Pha,
            AsmInst::Lda(Operand::StackRelative(4)),
            AsmInst::Pha,
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc16Index16),
                AsmInst::Lda(Operand::StackRelative(4)),
                AsmInst::Pha,
                AsmInst::Lda(Operand::StackRelative(4)),
                AsmInst::Pha,
            ]
        );
    }

    #[test]
    fn removes_redundant_direct_reload_after_redundant_mode() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc16Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Mode(Mode::Acc16Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Adc(Operand::Immediate(Immediate::Word(1))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc16Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
                AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
                AsmInst::Adc(Operand::Immediate(Immediate::Word(1))),
            ]
        );
    }

    #[test]
    fn removes_redundant_immediate_reload_after_store() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x41))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x41))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x41))),
                AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
            ]
        );
    }

    #[test]
    fn removes_redundant_direct_self_store_after_load() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("__dp_arg_0".to_owned()))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("__dp_arg_0".to_owned()))),
            AsmInst::Jsr("callee".to_owned()),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Direct(Symbol::Named("__dp_arg_0".to_owned()))),
                AsmInst::Jsr("callee".to_owned()),
            ]
        );
    }

    #[test]
    fn keeps_direct_store_when_accumulator_value_is_unknown() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Jsr("producer".to_owned()),
            AsmInst::Sta(Operand::Direct(Symbol::Named("__dp_arg_0".to_owned()))),
            AsmInst::Jsr("callee".to_owned()),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Jsr("producer".to_owned()),
                AsmInst::Sta(Operand::Direct(Symbol::Named("__dp_arg_0".to_owned()))),
                AsmInst::Jsr("callee".to_owned()),
            ]
        );
    }

    #[test]
    fn keeps_reload_after_flags_stop_matching_accumulator() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc16Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Cmp(Operand::Immediate(Immediate::Word(0))),
            AsmInst::Lda(Operand::Direct(Symbol::Named("slot".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc16Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
                AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
                AsmInst::Cmp(Operand::Immediate(Immediate::Word(0))),
                AsmInst::Lda(Operand::Direct(Symbol::Named("slot".to_owned()))),
            ]
        );
    }

    #[test]
    fn keeps_reload_after_label_boundary() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x41))),
            AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
            AsmInst::Label("entry".to_owned()),
            AsmInst::Lda(Operand::Direct(Symbol::Named("slot".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x41))),
                AsmInst::Sta(Operand::Direct(Symbol::Named("slot".to_owned()))),
                AsmInst::Label("entry".to_owned()),
                AsmInst::Lda(Operand::Direct(Symbol::Named("slot".to_owned()))),
            ]
        );
    }

    #[test]
    fn keeps_absolute_named_reload_after_store() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x41))),
            AsmInst::Sta(Operand::Absolute(Symbol::Named(
                "maybe_volatile".to_owned(),
            ))),
            AsmInst::Lda(Operand::Absolute(Symbol::Named(
                "maybe_volatile".to_owned(),
            ))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x41))),
                AsmInst::Sta(Operand::Absolute(Symbol::Named(
                    "maybe_volatile".to_owned()
                ))),
                AsmInst::Lda(Operand::Absolute(Symbol::Named(
                    "maybe_volatile".to_owned()
                ))),
            ]
        );
    }

    #[test]
    fn removes_redundant_index_transfer_before_indexed_load() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Tax,
            AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
            AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Tax,
            AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("other".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
                AsmInst::Tax,
                AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
                AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("other".to_owned()))),
            ]
        );
    }

    #[test]
    fn removes_redundant_index_transfer_after_redundant_mode() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Tax,
            AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Tax,
            AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("other".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
                AsmInst::Tax,
                AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
                AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("other".to_owned()))),
            ]
        );
    }

    #[test]
    fn removes_redundant_index_load_when_flags_still_match() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
            AsmInst::Phx,
            AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
            AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
                AsmInst::Phx,
                AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
            ]
        );
    }

    #[test]
    fn removes_redundant_y_load_when_flags_still_match() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Ldy(Operand::Direct(Symbol::Named("offset".to_owned()))),
            AsmInst::Phy,
            AsmInst::Ldy(Operand::Direct(Symbol::Named("offset".to_owned()))),
            AsmInst::Lda(Operand::IndirectDirectIndexedY(Symbol::Named(
                "ptr".to_owned(),
            ))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Ldy(Operand::Direct(Symbol::Named("offset".to_owned()))),
                AsmInst::Phy,
                AsmInst::Lda(Operand::IndirectDirectIndexedY(Symbol::Named(
                    "ptr".to_owned(),
                ))),
            ]
        );
    }

    #[test]
    fn keeps_redundant_index_load_after_flags_stop_matching() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(0))),
            AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
            AsmInst::Beq("done".to_owned()),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
                AsmInst::Lda(Operand::Immediate(Immediate::Byte(0))),
                AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
                AsmInst::Beq("done".to_owned()),
            ]
        );
    }

    #[test]
    fn keeps_redundant_index_load_after_other_index_load_sets_flags() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
            AsmInst::Ldy(Operand::Immediate(Immediate::Word(0))),
            AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
            AsmInst::Beq("done".to_owned()),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
                AsmInst::Ldy(Operand::Immediate(Immediate::Word(0))),
                AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
                AsmInst::Beq("done".to_owned()),
            ]
        );
    }

    #[test]
    fn keeps_redundant_index_load_after_label_boundary() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
            AsmInst::Label("entry".to_owned()),
            AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
            AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
                AsmInst::Label("entry".to_owned()),
                AsmInst::Ldx(Operand::Immediate(Immediate::Word(4))),
                AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
            ]
        );
    }

    #[test]
    fn keeps_index_transfer_after_index_source_store() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Tax,
            AsmInst::Stz(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Tax,
            AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
                AsmInst::Tax,
                AsmInst::Stz(Operand::Direct(Symbol::Named("index".to_owned()))),
                AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
                AsmInst::Tax,
                AsmInst::Lda(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
            ]
        );
    }

    #[test]
    fn keeps_index_transfer_before_indexed_store() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Tax,
            AsmInst::Lda(Operand::Direct(Symbol::Named("value".to_owned()))),
            AsmInst::Sta(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
            AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
            AsmInst::Tax,
            AsmInst::Sta(Operand::AbsoluteIndexedX(Symbol::Named("other".to_owned()))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
                AsmInst::Tax,
                AsmInst::Lda(Operand::Direct(Symbol::Named("value".to_owned()))),
                AsmInst::Sta(Operand::AbsoluteIndexedX(Symbol::Named("table".to_owned()))),
                AsmInst::Lda(Operand::Direct(Symbol::Named("index".to_owned()))),
                AsmInst::Tax,
                AsmInst::Sta(Operand::AbsoluteIndexedX(Symbol::Named("other".to_owned()))),
            ]
        );
    }

    #[test]
    fn shortens_local_brl_to_bra() {
        let module = module(vec![
            AsmInst::Brl("done".to_owned()),
            AsmInst::Nop,
            AsmInst::Label("done".to_owned()),
            AsmInst::Rts,
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Bra("done".to_owned()),
                AsmInst::Nop,
                AsmInst::Label("done".to_owned()),
                AsmInst::Rts,
            ]
        );
    }

    #[test]
    fn uses_per_for_same_routine_label_pushes() {
        let module = module(vec![
            AsmInst::Pea(Symbol::Named("target".to_owned())),
            AsmInst::Label("target".to_owned()),
            AsmInst::Rts,
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Per("target".to_owned()),
                AsmInst::Label("target".to_owned()),
                AsmInst::Rts,
            ]
        );
    }

    #[test]
    fn replaces_reloaded_immediate_pha_with_pea() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc16Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
            AsmInst::Pha,
            AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Pea(Symbol::Address(0x1234)),
                AsmInst::Mode(Mode::Acc16Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
            ]
        );
    }

    #[test]
    fn replaces_flag_preserving_xba_byte_compose_with_word_load() {
        let module = module(vec![
            AsmInst::Mode(Mode::Acc8Index16),
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x12))),
            AsmInst::Xba,
            AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x34))),
            AsmInst::Mode(Mode::Acc16Index16),
        ]);

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Mode(Mode::Acc16Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
            ]
        );
    }

    #[test]
    fn prefers_direct_operands_for_direct_symbols() {
        let mut module = module(vec![
            AsmInst::Lda(Operand::Absolute(Symbol::Named("slot".to_owned()))),
            AsmInst::Sta(Operand::AbsoluteIndexedX(Symbol::Named("slot".to_owned()))),
        ]);
        module.direct_symbols = vec![DirectSymbol {
            name: "slot".to_owned(),
            size: 1,
            export: false,
        }];

        assert_eq!(
            optimized_insts(&module),
            vec![
                AsmInst::Lda(Operand::Direct(Symbol::Named("slot".to_owned()))),
                AsmInst::Sta(Operand::DirectIndexedX(Symbol::Named("slot".to_owned()))),
            ]
        );
    }
}
