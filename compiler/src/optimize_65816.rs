use std::collections::{BTreeMap, BTreeSet};

use crate::target_65816::{AsmInst, AsmModule, AsmRoutine, Immediate, Mode, Operand, Symbol};

const SHORT_BRANCH_DISTANCE: usize = 120;

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
        fixed_symbols: module.fixed_symbols.clone(),
        banked_symbols: module.banked_symbols.clone(),
        vectors: module.vectors.clone(),
        metadata: module.metadata.clone(),
        imports: module.imports.clone(),
        libraries: module.libraries.clone(),
        external_routines: module.external_routines.clone(),
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

    AsmRoutine {
        name: routine.name.clone(),
        export: routine.export,
        insts,
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
        Operand::Absolute(_)
            | Operand::AbsoluteIndexedX(_)
            | Operand::Direct(_)
            | Operand::DirectIndexedX(_)
    )
}

fn inc_dec_operand(operand: &Operand) -> bool {
    matches!(
        operand,
        Operand::Absolute(_)
            | Operand::AbsoluteIndexedX(_)
            | Operand::Direct(_)
            | Operand::DirectIndexedX(_)
    )
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
