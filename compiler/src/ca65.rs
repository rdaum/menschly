use crate::target_65816::{AsmInst, AsmModule, AsmRoutine, Immediate, Operand, Symbol};

pub fn emit_module(module: &AsmModule) -> String {
    let mut out = String::new();
    out.push_str(".p816\n");

    if !module.direct_symbols.is_empty() {
        out.push_str(".segment \"ZEROPAGE\"\n");
        for symbol in &module.direct_symbols {
            out.push_str(&symbol.name);
            out.push_str(": .res ");
            out.push_str(&symbol.size.to_string());
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str(".code\n\n");

    for routine in &module.routines {
        emit_routine(&mut out, routine);
        out.push('\n');
    }

    out
}

fn emit_routine(out: &mut String, routine: &AsmRoutine) {
    if routine.export {
        out.push_str(".export ");
        out.push_str(&routine.name);
        out.push('\n');
    }
    out.push_str(".proc ");
    out.push_str(&routine.name);
    out.push('\n');

    for inst in &routine.insts {
        match inst {
            AsmInst::Label(label) => {
                out.push_str(label);
                out.push_str(":\n");
            }
            _ => {
                out.push('\t');
                out.push_str(&emit_inst(inst));
                out.push('\n');
            }
        }
    }

    out.push_str(".endproc\n");
}

fn emit_inst(inst: &AsmInst) -> String {
    match inst {
        AsmInst::Label(_) => unreachable!("labels are emitted by emit_routine"),
        AsmInst::Mode(mode) => mode.directives().join("\n\t"),
        AsmInst::Lda(operand) => one_operand("lda", operand),
        AsmInst::Ldx(operand) => one_operand("ldx", operand),
        AsmInst::Ldy(operand) => one_operand("ldy", operand),
        AsmInst::Sta(operand) => one_operand("sta", operand),
        AsmInst::Stx(operand) => one_operand("stx", operand),
        AsmInst::Sty(operand) => one_operand("sty", operand),
        AsmInst::Stz(operand) => one_operand("stz", operand),
        AsmInst::Tax => "tax".to_owned(),
        AsmInst::Tay => "tay".to_owned(),
        AsmInst::Txa => "txa".to_owned(),
        AsmInst::Tya => "tya".to_owned(),
        AsmInst::Inx => "inx".to_owned(),
        AsmInst::Clc => "clc".to_owned(),
        AsmInst::Sec => "sec".to_owned(),
        AsmInst::Adc(operand) => one_operand("adc", operand),
        AsmInst::Sbc(operand) => one_operand("sbc", operand),
        AsmInst::Inc(operand) => one_operand("inc", operand),
        AsmInst::Dec(operand) => one_operand("dec", operand),
        AsmInst::Cmp(operand) => one_operand("cmp", operand),
        AsmInst::Cpx(operand) => one_operand("cpx", operand),
        AsmInst::Beq(label) => branch("beq", label),
        AsmInst::Bne(label) => branch("bne", label),
        AsmInst::Bra(label) => branch("bra", label),
        AsmInst::Jsr(label) => branch("jsr", label),
        AsmInst::Jsl(label) => branch("jsl", label),
        AsmInst::Rts => "rts".to_owned(),
        AsmInst::Rtl => "rtl".to_owned(),
        AsmInst::Rti => "rti".to_owned(),
        AsmInst::Nop => "nop".to_owned(),
        AsmInst::Stp => "stp".to_owned(),
        AsmInst::Raw(line) => line.clone(),
    }
}

fn one_operand(opcode: &str, operand: &Operand) -> String {
    format!("{opcode} {}", emit_operand(operand))
}

fn branch(opcode: &str, label: &str) -> String {
    format!("{opcode} {label}")
}

fn emit_operand(operand: &Operand) -> String {
    match operand {
        Operand::Immediate(value) => emit_immediate(*value),
        Operand::Absolute(symbol) => emit_symbol(symbol),
        Operand::AbsoluteIndexedX(symbol) => format!("{},x", emit_symbol(symbol)),
        Operand::AbsoluteLong(symbol) => format!("f:{}", emit_symbol(symbol)),
        Operand::Direct(symbol) => emit_symbol(symbol),
        Operand::DirectIndexedX(symbol) => format!("{},x", emit_symbol(symbol)),
        Operand::DirectIndexedY(symbol) => format!("{},y", emit_symbol(symbol)),
        Operand::IndirectDirect(symbol) => format!("({})", emit_symbol(symbol)),
        Operand::IndirectDirectIndexedY(symbol) => format!("({}),y", emit_symbol(symbol)),
        Operand::IndirectLongDirect(symbol) => format!("[{}]", emit_symbol(symbol)),
        Operand::IndirectLongDirectIndexedY(symbol) => format!("[{}],y", emit_symbol(symbol)),
    }
}

fn emit_immediate(value: Immediate) -> String {
    match value {
        Immediate::Byte(value) => format!("#${value:02x}"),
        Immediate::Word(value) => format!("#${value:04x}"),
        Immediate::Long(value) => format!("#${value:06x}"),
    }
}

fn emit_symbol(symbol: &Symbol) -> String {
    match symbol {
        Symbol::Named(name) => name.clone(),
        Symbol::Address(address) if *address <= 0xffff => format!("${address:04x}"),
        Symbol::Address(address) => format!("${address:06x}"),
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures;
    use crate::lower_65816::lower_module;

    use super::emit_module;

    #[test]
    fn emits_exported_return_constant_fixture() {
        let module = lower_module(&fixtures::return_constant_u16()).unwrap();
        let asm = emit_module(&module);

        assert!(asm.contains(".p816\n"));
        assert!(asm.contains(".export return_constant_u16\n"));
        assert!(asm.contains(".proc return_constant_u16\n"));
        assert!(asm.contains("\trep #$30\n\t.a16\n\t.i16\n"));
        assert!(asm.contains("\tlda #$1234\n"));
        assert!(asm.contains("\trtl\n"));
    }

    #[test]
    fn emits_store_byte_fixture() {
        let module = lower_module(&fixtures::store_byte()).unwrap();
        let asm = emit_module(&module);

        assert!(asm.contains(".export store_byte\n"));
        assert!(asm.contains("\tsep #$20\n\t.a8\n\t.i16\n"));
        assert!(asm.contains("\tlda #$7f\n"));
        assert!(asm.contains("\tsta $040000\n"));
        assert!(asm.contains("\trtl\n"));
    }

    #[test]
    fn emits_branching_copy_fixture() {
        let module = lower_module(&fixtures::copy_bytes_near()).unwrap();
        let asm = emit_module(&module);

        assert!(asm.contains("copy_loop:\n"));
        assert!(asm.contains("\tlda $1000,x\n"));
        assert!(asm.contains("\tsta $1100,x\n"));
        assert!(asm.contains("\tinx\n"));
        assert!(asm.contains("\tbne copy_loop\n"));
    }

    #[test]
    fn emits_direct_page_symbols_for_storage_assigned_fixture() {
        let (module, _assignment) =
            crate::lower_65816::lower_module_with_storage(&fixtures::direct_local_store()).unwrap();
        let asm = emit_module(&module);

        assert!(asm.contains(".segment \"ZEROPAGE\"\n"));
        assert!(asm.contains("__dp_direct_local_store_local_tmp: .res 1\n"));
        assert!(asm.contains("\tsta __dp_direct_local_store_local_tmp\n"));
    }
}
