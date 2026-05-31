use crate::target_65816::{
    AsmInst, AsmModule, AsmRoutine, Immediate, Mode, Operand, Symbol,
};

pub fn return_constant_u16() -> AsmModule {
    AsmModule::new("return_constant_u16").with_routine(
        AsmRoutine::new("return_constant_u16")
            .exported()
            .with_insts(vec![
                AsmInst::Mode(Mode::Acc16Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Word(0x1234))),
                AsmInst::Rtl,
            ]),
    )
}

pub fn store_byte() -> AsmModule {
    AsmModule::new("store_byte").with_routine(
        AsmRoutine::new("store_byte")
            .exported()
            .with_insts(vec![
                AsmInst::Mode(Mode::Acc8Index16),
                AsmInst::Lda(Operand::Immediate(Immediate::Byte(0x7f))),
                AsmInst::Sta(Operand::Absolute(Symbol::Address(0x040000))),
                AsmInst::Rtl,
            ]),
    )
}
