use crate::ir::{
    Abi, Address, Const, IrInst, Label, Local, Module, Routine, Storage, Type, Value, Width,
};

pub const COPY_SRC: u32 = 0x1000;
pub const COPY_DST: u32 = 0x1100;
pub const CLEAR_DST: u32 = 0x1200;
pub const STORE_BYTE_ADDR: u32 = 0x040000;
pub const HELPER_STORE_ADDR: u32 = 0x040001;
pub const FUNCTION_STORE_ADDR: u32 = 0x040002;
pub const MAIN_STORE_ADDR: u32 = 0x040003;
pub const DIRECT_STORE_ADDR: u32 = 0x040004;

pub fn return_constant_u16() -> Module {
    Module::new("return_constant_u16").with_routine(
        Routine::function("return_constant_u16", Type::U16)
            .exported()
            .with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Accumulator,
                    value: Const::U16(0x1234),
                },
                IrInst::ReturnValue(Value::Accumulator),
            ]),
    )
}

pub fn store_byte() -> Module {
    Module::new("store_byte").with_routine(Routine::procedure("store_byte").exported().with_body(
        vec![
            IrInst::LoadConst {
                dst: Value::Accumulator,
                value: Const::U8(0x7f),
            },
            IrInst::Store {
                width: Width::Byte,
                addr: Address::Absolute(STORE_BYTE_ADDR),
                src: Value::Accumulator,
            },
            IrInst::Return,
        ],
    ))
}

pub fn copy_bytes_near() -> Module {
    let loop_label = Label::new("copy_loop");

    Module::new("copy_bytes_near").with_routine(
        Routine::procedure("copy_bytes_near")
            .exported()
            .with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Accumulator,
                    value: Const::U8(0),
                },
                IrInst::LoadConst {
                    dst: Value::IndexX,
                    value: Const::U16(0),
                },
                IrInst::Label(loop_label.clone()),
                IrInst::Load {
                    width: Width::Byte,
                    dst: Value::Accumulator,
                    addr: Address::AbsoluteIndexedX(COPY_SRC),
                },
                IrInst::Store {
                    width: Width::Byte,
                    addr: Address::AbsoluteIndexedX(COPY_DST),
                    src: Value::Accumulator,
                },
                IrInst::Inc {
                    width: Width::Word,
                    value: Value::IndexX,
                },
                IrInst::Compare {
                    width: Width::Word,
                    lhs: Value::IndexX,
                    rhs: Value::Const(Const::U16(4)),
                },
                IrInst::BranchIfNotEqual { target: loop_label },
                IrInst::Return,
            ]),
    )
}

pub fn clear_memory() -> Module {
    let loop_label = Label::new("clear_loop");

    Module::new("clear_memory").with_routine(
        Routine::procedure("clear_memory")
            .exported()
            .with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Accumulator,
                    value: Const::U8(0),
                },
                IrInst::LoadConst {
                    dst: Value::IndexX,
                    value: Const::U16(0),
                },
                IrInst::Label(loop_label.clone()),
                IrInst::Store {
                    width: Width::Byte,
                    addr: Address::AbsoluteIndexedX(CLEAR_DST),
                    src: Value::Accumulator,
                },
                IrInst::Inc {
                    width: Width::Word,
                    value: Value::IndexX,
                },
                IrInst::Compare {
                    width: Width::Word,
                    lhs: Value::IndexX,
                    rhs: Value::Const(Const::U16(4)),
                },
                IrInst::BranchIfNotEqual { target: loop_label },
                IrInst::Return,
            ]),
    )
}

pub fn call_internal_procedure() -> Module {
    Module::new("call_internal_procedure")
        .with_routine(
            Routine::procedure("call_internal_procedure")
                .exported()
                .with_body(vec![
                    IrInst::CallProcedure {
                        name: "store_helper".to_owned(),
                        abi: Abi::Internal,
                    },
                    IrInst::Return,
                ]),
        )
        .with_routine(Routine::procedure("store_helper").with_body(vec![
            IrInst::LoadConst {
                dst: Value::Accumulator,
                value: Const::U8(0x42),
            },
            IrInst::Store {
                width: Width::Byte,
                addr: Address::Absolute(HELPER_STORE_ADDR),
                src: Value::Accumulator,
            },
            IrInst::Return,
        ]))
}

pub fn call_internal_function() -> Module {
    Module::new("call_internal_function")
        .with_routine(
            Routine::procedure("call_internal_function")
                .exported()
                .with_body(vec![
                    IrInst::CallFunction {
                        name: "value_helper".to_owned(),
                        abi: Abi::Internal,
                        dst: Value::Accumulator,
                    },
                    IrInst::Store {
                        width: Width::Byte,
                        addr: Address::Absolute(FUNCTION_STORE_ADDR),
                        src: Value::Accumulator,
                    },
                    IrInst::Return,
                ]),
        )
        .with_routine(Routine::function("value_helper", Type::U8).with_body(vec![
            IrInst::LoadConst {
                dst: Value::Accumulator,
                value: Const::U8(0x55),
            },
            IrInst::ReturnValue(Value::Accumulator),
        ]))
}

pub fn exported_main() -> Module {
    Module::new("exported_main").with_routine(Routine::procedure("main").exported().with_body(
        vec![
            IrInst::LoadConst {
                dst: Value::Accumulator,
                value: Const::U8(0x99),
            },
            IrInst::Store {
                width: Width::Byte,
                addr: Address::Absolute(MAIN_STORE_ADDR),
                src: Value::Accumulator,
            },
            IrInst::Return,
        ],
    ))
}

pub fn direct_local_store() -> Module {
    Module::new("direct_local_store").with_routine(
        Routine::procedure("direct_local_store")
            .exported()
            .with_local(Local::new("tmp", Type::U8, Storage::Direct))
            .with_body(vec![
                IrInst::LoadConst {
                    dst: Value::Local("tmp".to_owned()),
                    value: Const::U8(0x66),
                },
                IrInst::Store {
                    width: Width::Byte,
                    addr: Address::Absolute(DIRECT_STORE_ADDR),
                    src: Value::Local("tmp".to_owned()),
                },
                IrInst::Return,
            ]),
    )
}
