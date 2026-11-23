use crate::target_65816::{AsmInst, AsmModule, Immediate, Operand, Symbol};

/// An instruction type that can be printed as ca65 source.
pub trait Ca65Inst {
    /// The `.setcpu`-style directive that starts every module for this target.
    const CPU_DIRECTIVE: &'static str;

    /// Returns the label name if this instruction is a label definition.
    fn label(&self) -> Option<&str>;

    /// Returns the instruction text (may span several lines, tab-separated).
    fn emit(&self) -> String;
}

impl Ca65Inst for AsmInst {
    const CPU_DIRECTIVE: &'static str = ".p816";

    fn label(&self) -> Option<&str> {
        match self {
            AsmInst::Label(label) => Some(label),
            _ => None,
        }
    }

    fn emit(&self) -> String {
        emit_inst(self)
    }
}

/// Emits a 65816 module. Kept as the familiar entry point for 65816 callers.
pub fn emit_module(module: &AsmModule) -> String {
    emit(module)
}

/// Emits a module for any target whose instructions implement [`Ca65Inst`].
pub fn emit<I: Ca65Inst>(module: &crate::asm::AsmModule<I>) -> String {
    let mut out = String::new();
    out.push_str(I::CPU_DIRECTIVE);
    out.push('\n');

    for routine in &module.external_routines {
        out.push_str(".import ");
        out.push_str(routine);
        out.push('\n');
    }
    for symbol in &module.external_direct_symbols {
        out.push_str(".importzp ");
        out.push_str(symbol);
        out.push('\n');
    }
    for data in &module.external_data {
        out.push_str(".import ");
        out.push_str(&data.name);
        if data.far {
            out.push_str(": far");
        }
        out.push('\n');
    }
    if !module.external_routines.is_empty()
        || !module.external_direct_symbols.is_empty()
        || !module.external_data.is_empty()
    {
        out.push('\n');
    }

    for symbol in &module.fixed_symbols {
        if symbol.export {
            out.push_str(".export ");
            out.push_str(&symbol.name);
            out.push('\n');
        }
        out.push_str(&symbol.name);
        out.push_str(" = ");
        out.push_str(&emit_symbol(&Symbol::Address(symbol.address)));
        out.push('\n');
    }
    if !module.fixed_symbols.is_empty() {
        out.push('\n');
    }

    if !module.direct_symbols.is_empty() {
        out.push_str(".segment \"ZEROPAGE\"\n");
        for symbol in &module.direct_symbols {
            if symbol.export {
                out.push_str(".exportzp ");
                out.push_str(&symbol.name);
                out.push('\n');
            }
            out.push_str(&symbol.name);
            match &symbol.alias {
                Some((base, offset)) => {
                    out.push_str(&format!(" = {base}+{offset}"));
                }
                None => {
                    out.push_str(": .res ");
                    out.push_str(&symbol.size.to_string());
                }
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if !module.static_symbols.is_empty() {
        out.push_str(".bss\n");
        for symbol in &module.static_symbols {
            if symbol.export {
                out.push_str(".export ");
                out.push_str(&symbol.name);
                out.push('\n');
            }
            out.push_str(&symbol.name);
            out.push_str(": .res ");
            out.push_str(&symbol.size.to_string());
            out.push('\n');
        }
        out.push('\n');
    }

    let code_segment = code_segment(module);
    let data_segment = data_segment(module).unwrap_or(code_segment);

    for symbol in &module.banked_symbols {
        out.push_str(".segment \"BANK");
        out.push_str(&symbol.bank.to_string());
        out.push_str("\"\n");
        if symbol.export {
            out.push_str(".export ");
            out.push_str(&symbol.name);
            out.push('\n');
        }
        out.push_str(&symbol.name);
        out.push_str(": .res ");
        out.push_str(&symbol.size.to_string());
        out.push('\n');
        out.push('\n');
    }

    if !module.vectors.is_empty() {
        for vector in &module.vectors {
            out.push_str(".segment \"");
            out.push_str(&vector_segment(&vector.name));
            out.push_str("\"\n");
            out.push_str("__vector_");
            out.push_str(&vector.name);
            out.push_str(": .addr ");
            out.push_str(&vector.routine);
            out.push_str("\n\n");
        }
    }

    if has_module_metadata(module) {
        out.push_str(".segment \"MENSCH_META\"\n");
        emit_module_metadata(&mut out, module);
        out.push('\n');
    }

    emit_code_segment(&mut out, code_segment);
    out.push('\n');

    for routine in &module.routines {
        if let Some(segment) = &routine.segment {
            emit_code_segment(&mut out, segment);
        }
        emit_routine(&mut out, routine);
        if routine.segment.is_some() {
            emit_code_segment(&mut out, code_segment);
        }
        out.push('\n');
    }

    // Data after the code, so a module's first routine starts its segment.
    if !module.static_data.is_empty() {
        emit_code_segment(&mut out, data_segment);
        for data in &module.static_data {
            if data.export {
                out.push_str(".export ");
                out.push_str(&data.name);
                out.push('\n');
            }
            out.push_str(&data.name);
            out.push_str(":\n");
            for item in &data.items {
                match item {
                    crate::ir::DataItem::Bytes(bytes) => emit_bytes(&mut out, bytes),
                    crate::ir::DataItem::Address { symbol, bytes: 4 } => {
                        out.push_str("\t.dword ");
                        out.push_str(symbol);
                        out.push('\n');
                    }
                    crate::ir::DataItem::Address { symbol, .. } => {
                        out.push_str("\t.word .loword(");
                        out.push_str(symbol);
                        out.push_str(")\n");
                    }
                }
            }
        }
        out.push('\n');
    }

    out
}

fn code_segment<I>(module: &crate::asm::AsmModule<I>) -> &str {
    module
        .metadata
        .iter()
        .find(|metadata| metadata.key == "code_segment")
        .map(|metadata| metadata.value.as_str())
        .unwrap_or("CODE")
}

fn data_segment<I>(module: &crate::asm::AsmModule<I>) -> Option<&str> {
    module
        .metadata
        .iter()
        .find(|metadata| metadata.key == "data_segment")
        .map(|metadata| metadata.value.as_str())
}

fn emit_code_segment(out: &mut String, segment: &str) {
    if segment == "CODE" {
        out.push_str(".code\n");
    } else {
        out.push_str(".segment \"");
        out.push_str(segment);
        out.push_str("\"\n");
    }
}

fn vector_segment(name: &str) -> String {
    match name {
        "cop" => "VECTOR_COP".to_owned(),
        "brk" => "VECTOR_BRK".to_owned(),
        "abort" => "VECTOR_ABORT".to_owned(),
        "nmi" => "VECTOR_NMI".to_owned(),
        "irq" => "VECTOR_IRQ".to_owned(),
        // Trap table entries: `trap_0d` -> `VECTOR_TRAP_0D`.
        _ if name.starts_with("trap_") => format!("VECTOR_{}", name.to_ascii_uppercase()),
        _ => "VECTORS".to_owned(),
    }
}

fn has_module_metadata<I>(module: &crate::asm::AsmModule<I>) -> bool {
    module.entry.is_some()
        || !module.imports.is_empty()
        || !module.libraries.is_empty()
        || !module.metadata.is_empty()
}

fn emit_module_metadata<I>(out: &mut String, module: &crate::asm::AsmModule<I>) {
    if let Some(entry) = &module.entry {
        emit_metadata(out, &format!("entry.{}={}", entry.kind, entry.routine));
    }
    for import in &module.imports {
        emit_metadata(out, &format!("import={}", import.module));
    }
    for library in &module.libraries {
        emit_metadata(out, &format!("library={}", library.name));
    }
    for metadata in &module.metadata {
        emit_metadata(out, &format!("{}={}", metadata.key, metadata.value));
    }
}

fn emit_metadata(out: &mut String, value: &str) {
    out.push_str("\t.asciiz ");
    out.push_str(&quoted(value));
    out.push('\n');
}

fn emit_bytes(out: &mut String, bytes: &[u8]) {
    for chunk in bytes.chunks(16) {
        out.push_str("\t.byte ");
        for (idx, byte) in chunk.iter().enumerate() {
            if idx > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("${byte:02x}"));
        }
        out.push('\n');
    }
}

fn quoted(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn emit_routine<I: Ca65Inst>(out: &mut String, routine: &crate::asm::AsmRoutine<I>) {
    if routine.export {
        out.push_str(".export ");
        out.push_str(&routine.name);
        out.push('\n');
    }
    out.push_str(".proc ");
    out.push_str(&routine.name);
    out.push('\n');

    for inst in &routine.insts {
        if let Some(label) = inst.label() {
            out.push_str(label);
            out.push_str(":\n");
        } else {
            out.push('\t');
            out.push_str(&inst.emit());
            out.push('\n');
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
        AsmInst::Tsc => "tsc".to_owned(),
        AsmInst::Tcs => "tcs".to_owned(),
        AsmInst::Inx => "inx".to_owned(),
        AsmInst::Clc => "clc".to_owned(),
        AsmInst::Sec => "sec".to_owned(),
        AsmInst::Adc(operand) => one_operand("adc", operand),
        AsmInst::Sbc(operand) => one_operand("sbc", operand),
        AsmInst::And(operand) => one_operand("and", operand),
        AsmInst::Ora(operand) => one_operand("ora", operand),
        AsmInst::Eor(operand) => one_operand("eor", operand),
        AsmInst::AslA => "asl a".to_owned(),
        AsmInst::LsrA => "lsr a".to_owned(),
        AsmInst::Xba => "xba".to_owned(),
        AsmInst::Pha => "pha".to_owned(),
        AsmInst::Phx => "phx".to_owned(),
        AsmInst::Phy => "phy".to_owned(),
        AsmInst::Php => "php".to_owned(),
        AsmInst::Phb => "phb".to_owned(),
        AsmInst::Phk => "phk".to_owned(),
        AsmInst::Pea(symbol) => format!("pea {}", emit_symbol(symbol)),
        AsmInst::Per(label) => branch("per", label),
        AsmInst::Pla => "pla".to_owned(),
        AsmInst::Plx => "plx".to_owned(),
        AsmInst::Ply => "ply".to_owned(),
        AsmInst::Plp => "plp".to_owned(),
        AsmInst::Plb => "plb".to_owned(),
        AsmInst::Inc(operand) => one_operand("inc", operand),
        AsmInst::Dec(operand) => one_operand("dec", operand),
        AsmInst::Cmp(operand) => one_operand("cmp", operand),
        AsmInst::Cpx(operand) => one_operand("cpx", operand),
        AsmInst::Beq(label) => branch("beq", label),
        AsmInst::Bne(label) => branch("bne", label),
        AsmInst::Bcc(label) => branch("bcc", label),
        AsmInst::Bcs(label) => branch("bcs", label),
        AsmInst::Bra(label) => branch("bra", label),
        AsmInst::Brl(label) => branch("brl", label),
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
        Operand::ImmediateSymbol(symbol) => format!("#{}", emit_symbol(symbol)),
        Operand::Absolute(symbol) => emit_symbol(symbol),
        Operand::AbsoluteIndexedX(symbol) => format!("{},x", emit_symbol(symbol)),
        Operand::AbsoluteLong(symbol) => format!("f:{}", emit_symbol(symbol)),
        Operand::Direct(symbol) => emit_symbol(symbol),
        Operand::DirectIndexedX(symbol) => format!("{},x", emit_symbol(symbol)),
        Operand::DirectIndexedY(symbol) => format!("{},y", emit_symbol(symbol)),
        Operand::StackRelative(offset) => format!("${offset:02x},s"),
        Operand::StackRelativeIndirectIndexedY(offset) => format!("(${offset:02x},s),y"),
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
    use crate::target_65816::{AsmInst, AsmModule, AsmRoutine, Symbol};

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
        assert!(asm.contains("\trts\n"));
    }

    #[test]
    fn emits_store_byte_fixture() {
        let module = lower_module(&fixtures::store_byte()).unwrap();
        let asm = emit_module(&module);

        assert!(asm.contains(".export store_byte\n"));
        assert!(asm.contains("\tsep #$20\n\t.a8\n\t.i16\n"));
        assert!(asm.contains("\tlda #$7f\n"));
        assert!(asm.contains("\tsta $040000\n"));
        assert!(asm.contains("\trts\n"));
    }

    #[test]
    fn emits_branching_copy_fixture() {
        let module = lower_module(&fixtures::copy_bytes_near()).unwrap();
        let asm = emit_module(&module);

        assert!(asm.contains("copy_loop:\n"));
        assert!(asm.contains("\tlda $1000,x\n"));
        assert!(asm.contains("\tsta $1100,x\n"));
        assert!(asm.contains("\tinx\n"));
        assert!(asm.contains("\tbeq :+\n\tbrl copy_loop\n:\n"));
    }

    #[test]
    fn emits_direct_page_symbols_for_storage_assigned_fixture() {
        let (module, _assignment) =
            crate::lower_65816::lower_module_with_storage(&fixtures::direct_local_store()).unwrap();
        let asm = emit_module(&module);

        assert!(asm.contains(".segment \"ZEROPAGE\"\n"));
        // A slot only one routine uses lives in the module's overlay region.
        assert!(asm.contains("__dp_overlay: .res 1\n"));
        assert!(asm.contains("__dp_direct_local_store_local_tmp = __dp_overlay+0\n"));
        assert!(asm.contains("\tsta __dp_direct_local_store_local_tmp\n"));
    }

    #[test]
    fn emits_native_65816_stack_and_branch_forms() {
        let module = AsmModule::new("native_forms").with_routine(
            AsmRoutine::new("native_forms").with_insts(vec![
                AsmInst::Xba,
                AsmInst::Pha,
                AsmInst::Pea(Symbol::Address(0x1234)),
                AsmInst::Per("nearby".to_owned()),
                AsmInst::Brl("farther".to_owned()),
            ]),
        );
        let asm = emit_module(&module);

        assert!(asm.contains("\txba\n"));
        assert!(asm.contains("\tpha\n"));
        assert!(asm.contains("\tpea $1234\n"));
        assert!(asm.contains("\tper nearby\n"));
        assert!(asm.contains("\tbrl farther\n"));
    }
}
