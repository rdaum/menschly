use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub globals: Vec<Global>,
    pub static_data: Vec<StaticData>,
    pub vectors: Vec<Vector>,
    pub metadata: Vec<Metadata>,
    pub imports: Vec<Import>,
    pub libraries: Vec<Library>,
    pub external_routines: Vec<String>,
    pub external_direct_symbols: Vec<String>,
    pub entry: Option<Entry>,
    pub routines: Vec<Routine>,
}

impl Module {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            globals: Vec::new(),
            static_data: Vec::new(),
            vectors: Vec::new(),
            metadata: Vec::new(),
            imports: Vec::new(),
            libraries: Vec::new(),
            external_routines: Vec::new(),
            external_direct_symbols: Vec::new(),
            entry: None,
            routines: Vec::new(),
        }
    }

    pub fn with_global(mut self, global: Global) -> Self {
        self.globals.push(global);
        self
    }

    pub fn with_vector(mut self, vector: Vector) -> Self {
        self.vectors.push(vector);
        self
    }

    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata.push(metadata);
        self
    }

    pub fn with_import(mut self, import: Import) -> Self {
        self.imports.push(import);
        self
    }

    pub fn with_library(mut self, library: Library) -> Self {
        self.libraries.push(library);
        self
    }

    pub fn with_entry(mut self, entry: Entry) -> Self {
        self.entry = Some(entry);
        self
    }

    pub fn with_routine(mut self, routine: Routine) -> Self {
        self.routines.push(routine);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticData {
    pub name: String,
    pub items: Vec<DataItem>,
    pub export: bool,
}

impl StaticData {
    pub fn bytes(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            items: vec![DataItem::Bytes(bytes)],
            export: false,
        }
    }
}

/// Initialised data: literal bytes, or the address of a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataItem {
    Bytes(Vec<u8>),
    /// A 2-byte (bank-relative) or 4-byte (flat) address.
    Address {
        symbol: String,
        bytes: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Global {
    pub name: String,
    pub ty: Type,
    pub placement: GlobalPlacement,
    pub volatile: bool,
    pub export: bool,
    /// Initial contents (in the module's data), for an initialised variable
    /// or a constant table; `None` leaves it in BSS.
    pub init: Option<Vec<DataItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalPlacement {
    Static,
    Fixed(u32),
    Bank(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vector {
    pub name: String,
    pub routine: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub module: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub routine: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Application,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    pub name: String,
    pub kind: RoutineKind,
    pub export: bool,
    pub abi: Abi,
    pub interrupt_preserve: InterruptPreserve,
    pub interrupt_raw: bool,
    pub interrupt_cop: bool,
    pub params: Vec<Param>,
    pub locals: Vec<Local>,
    pub stack_frame_bytes: u16,
    pub body: Vec<IrInst>,
}

impl Routine {
    pub fn procedure(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: RoutineKind::Procedure,
            export: false,
            abi: Abi::Internal,
            interrupt_preserve: InterruptPreserve::default_for_abi(Abi::Internal),
            interrupt_raw: false,
            interrupt_cop: false,
            params: Vec::new(),
            locals: Vec::new(),
            stack_frame_bytes: 0,
            body: Vec::new(),
        }
    }

    pub fn function(name: impl Into<String>, result: Type) -> Self {
        Self {
            name: name.into(),
            kind: RoutineKind::Function { result },
            export: false,
            abi: Abi::Internal,
            interrupt_preserve: InterruptPreserve::default_for_abi(Abi::Internal),
            interrupt_raw: false,
            interrupt_cop: false,
            params: Vec::new(),
            locals: Vec::new(),
            stack_frame_bytes: 0,
            body: Vec::new(),
        }
    }

    pub fn exported(mut self) -> Self {
        self.export = true;
        self.abi = Abi::Exported;
        self
    }

    pub fn public(mut self) -> Self {
        self.export = true;
        self
    }

    pub fn with_param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    pub fn with_local(mut self, local: Local) -> Self {
        self.locals.push(local);
        self
    }

    pub fn with_body(mut self, body: Vec<IrInst>) -> Self {
        self.body = body;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineKind {
    Procedure,
    Function { result: Type },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abi {
    Internal,
    Exported,
    Far,
    Interrupt,
    Reentrant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptPreserve {
    pub a: bool,
    pub x: bool,
    pub y: bool,
    pub p: bool,
    pub dbr: bool,
}

impl InterruptPreserve {
    pub const NONE: Self = Self {
        a: false,
        x: false,
        y: false,
        p: false,
        dbr: false,
    };

    pub const DEFAULT: Self = Self {
        a: true,
        x: true,
        y: true,
        p: true,
        dbr: true,
    };

    pub fn default_for_abi(abi: Abi) -> Self {
        if matches!(abi, Abi::Interrupt) {
            Self::DEFAULT
        } else {
            Self::NONE
        }
    }

    pub fn is_empty(self) -> bool {
        !self.a && !self.x && !self.y && !self.p && !self.dbr
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub storage: Storage,
}

pub fn param_symbol(routine: &str, name: &str) -> String {
    format!("__dp_{routine}_param_{name}")
}

impl Param {
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
        Self {
            name: name.into(),
            ty,
            storage: Storage::Default,
        }
    }

    pub fn direct(mut self) -> Self {
        self.storage = Storage::Direct;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub name: String,
    pub ty: Type,
    pub storage: Storage,
}

impl Local {
    pub fn new(name: impl Into<String>, ty: Type, storage: Storage) -> Self {
        Self {
            name: name.into(),
            ty,
            storage,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    U8,
    I8,
    U16,
    I16,
    U24,
    U32,
    I32,
    Pointer {
        to: Box<Type>,
        kind: PointerKind,
    },
    /// A pointer to a routine with this signature (a 16-bit code address).
    Routine(Box<RoutineType>),
    Record {
        fields: Vec<Field>,
    },
    Array {
        len: u16,
        element: Box<Type>,
    },
    Slice {
        element: Box<Type>,
    },
    Result {
        ok: Box<Type>,
        err: Box<Type>,
    },
}

/// The signature a routine pointer calls through. Only two calling
/// conventions can be called indirectly: routines without parameters
/// ([`Abi::Internal`]), and reentrant routines, whose arguments go on the
/// stack. Other routines take their arguments in storage named after the
/// routine, which an indirect caller cannot know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineType {
    pub abi: Abi,
    pub params: Vec<Type>,
    pub result: Option<Type>,
}

impl Type {
    pub fn size_bytes(&self) -> u16 {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U24 => 3,
            // Far pointers are 32-bit flat addresses: the MEGA65 has a 28-bit
            // address space; the 65816 uses the low three bytes.
            Self::U32
            | Self::I32
            | Self::Pointer {
                kind: PointerKind::Far,
                ..
            } => 4,
            Self::Pointer { .. } | Self::Routine(_) => 2,
            Self::Record { fields } => fields.iter().map(|field| field.ty.size_bytes()).sum(),
            Self::Array { len, element } => len.saturating_mul(element.size_bytes()),
            Self::Slice { .. } => 4,
            Self::Result { ok, err } => ok.size_bytes().max(err.size_bytes()) + 1,
        }
    }

    /// Whether arithmetic on this type is two's-complement signed.
    pub fn is_signed(&self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32)
    }

    pub fn near_ptr(to: Type) -> Self {
        Self::Pointer {
            to: Box::new(to),
            kind: PointerKind::Near,
        }
    }

    pub fn far_ptr(to: Type) -> Self {
        Self::Pointer {
            to: Box::new(to),
            kind: PointerKind::Far,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub offset: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Direct,
    Near,
    Far,
    Code,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Default,
    Static,
    Direct,
    Scratch,
    Stack,
    Far,
    Bank(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrInst {
    Label(Label),
    InlineAsm(String),
    Move {
        width: Width,
        dst: Value,
        src: Value,
    },
    LoadConst {
        dst: Value,
        value: Const,
    },
    LoadAddress {
        dst: Value,
        symbol: String,
        width: Width,
    },
    Load {
        width: Width,
        dst: Value,
        addr: Address,
    },
    Store {
        width: Width,
        addr: Address,
        src: Value,
    },
    Add {
        width: Width,
        dst: Value,
        lhs: Value,
        rhs: Value,
    },
    Sub {
        width: Width,
        dst: Value,
        lhs: Value,
        rhs: Value,
    },
    BitAnd {
        width: Width,
        dst: Value,
        lhs: Value,
        rhs: Value,
    },
    BitOr {
        width: Width,
        dst: Value,
        lhs: Value,
        rhs: Value,
    },
    /// `lhs << rhs`; `rhs` is always a constant bit count.
    ShiftLeft {
        width: Width,
        dst: Value,
        lhs: Value,
        rhs: Value,
    },
    /// Logical `lhs >> rhs`; `rhs` is always a constant bit count.
    ShiftRight {
        width: Width,
        dst: Value,
        lhs: Value,
        rhs: Value,
    },
    /// The arithmetic the CPUs have no single instruction for.
    Binary {
        op: BinaryOp,
        width: Width,
        dst: Value,
        lhs: Value,
        rhs: Value,
    },
    /// `dst = src` widened from `src_width` to `width`, copying the sign bit
    /// into the new bytes.
    SignExtend {
        width: Width,
        src_width: Width,
        dst: Value,
        src: Value,
    },
    Dec {
        width: Width,
        value: Value,
    },
    Inc {
        width: Width,
        value: Value,
    },
    /// Sets the flags for the branches below: Z when `lhs == rhs`, carry
    /// when `lhs >= rhs` (as two's-complement numbers when `signed`).
    Compare {
        width: Width,
        signed: bool,
        lhs: Value,
        rhs: Value,
    },
    Branch {
        target: Label,
    },
    BranchIfZero {
        value: Value,
        target: Label,
    },
    BranchIfEqual {
        target: Label,
    },
    BranchIfNotEqual {
        target: Label,
    },
    BranchIfCarrySet {
        target: Label,
    },
    BranchIfCarryClear {
        target: Label,
    },
    Push {
        width: Width,
        value: Value,
    },
    CallProcedure {
        name: String,
        abi: Abi,
    },
    CallFunction {
        name: String,
        abi: Abi,
        /// Width of the function result, so backends need not rely on
        /// machine state left behind by the callee.
        width: Width,
        dst: Value,
    },
    /// Calls the routine whose address is in `target` (a routine pointer).
    /// `result` is the width and destination of a function's result.
    CallIndirect {
        target: Value,
        abi: Abi,
        result: Option<(Width, Value)>,
    },
    CleanupStack {
        bytes: u16,
    },
    CopSignature {
        dst: Value,
    },
    CopRegister {
        register: CopRegister,
        dst: Value,
    },
    Trap {
        family: u8,
        call: Value,
        x: Value,
        y: Value,
        dst: Value,
    },
    Return,
    ReturnValue(Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopRegister {
    A,
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Mul,
    /// Division truncates toward zero; the remainder has the dividend's sign.
    Div {
        signed: bool,
    },
    Rem {
        signed: bool,
    },
    Xor,
    /// Arithmetic `lhs >> rhs`; `rhs` is always a constant bit count.
    ShiftRightSigned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
    Long,
    Quad,
}

impl Width {
    pub fn size_bytes(self) -> u16 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Long => 3,
            Self::Quad => 4,
        }
    }

    pub fn from_bytes(bytes: u16) -> Option<Self> {
        match bytes {
            1 => Some(Self::Byte),
            2 => Some(Self::Word),
            3 => Some(Self::Long),
            4 => Some(Self::Quad),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    Symbol(String),
    Direct(String),
    SymbolIndexedX(String),
    /// `base + index`, where `index` is a 16-bit byte offset. Each backend
    /// picks its own indexing strategy (X register, computed pointer, ...).
    Indexed {
        base: String,
        index: Value,
        /// The array spans at most 256 bytes, so (for any in-bounds index)
        /// the byte offset fits in its low byte.
        small: bool,
    },
    ValueOffset {
        base: Value,
        offset: u16,
    },
    NearPtr(Value),
    FarPtr(Value),
    Absolute(u32),
    AbsoluteIndexedX(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Accumulator,
    IndexX,
    IndexY,
    Const(Const),
    Temp(String),
    Local(String),
    Param(String),
    Direct(String),
    Stack { offset: u16, size: u16 },
    Global(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Const {
    U8(u8),
    U16(u16),
    U24(u32),
    U32(u32),
}

impl Const {
    /// The constant's bits, zero-extended.
    pub fn bits(self) -> u32 {
        match self {
            Self::U8(value) => u32::from(value),
            Self::U16(value) => u32::from(value),
            Self::U24(value) | Self::U32(value) => value,
        }
    }

    pub fn width(self) -> Width {
        match self {
            Self::U8(_) => Width::Byte,
            Self::U16(_) => Width::Word,
            Self::U24(_) => Width::Long,
            Self::U32(_) => Width::Quad,
        }
    }

    /// Byte `i` of the constant (zero above its width).
    pub fn byte(self, i: u16) -> u8 {
        self.bits().checked_shr(u32::from(i) * 8).unwrap_or(0) as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label(pub String);

impl Label {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
