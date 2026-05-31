use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub globals: Vec<Global>,
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
pub struct Global {
    pub name: String,
    pub ty: Type,
    pub placement: GlobalPlacement,
    pub volatile: bool,
    pub export: bool,
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
    pub params: Vec<Param>,
    pub locals: Vec<Local>,
    pub body: Vec<IrInst>,
}

impl Routine {
    pub fn procedure(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: RoutineKind::Procedure,
            export: false,
            abi: Abi::Internal,
            params: Vec::new(),
            locals: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn function(name: impl Into<String>, result: Type) -> Self {
        Self {
            name: name.into(),
            kind: RoutineKind::Function { result },
            export: false,
            abi: Abi::Internal,
            params: Vec::new(),
            locals: Vec::new(),
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
    Pointer { to: Box<Type>, kind: PointerKind },
    Record { fields: Vec<Field> },
    Array { len: u16, element: Box<Type> },
    Slice { element: Box<Type> },
    Result { ok: Box<Type>, err: Box<Type> },
}

impl Type {
    pub fn size_bytes(&self) -> u16 {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U24
            | Self::Pointer {
                kind: PointerKind::Far,
                ..
            } => 3,
            Self::Pointer { .. } => 2,
            Self::Record { fields } => fields.iter().map(|field| field.ty.size_bytes()).sum(),
            Self::Array { len, element } => len.saturating_mul(element.size_bytes()),
            Self::Slice { .. } => 4,
            Self::Result { ok, err } => ok.size_bytes().max(err.size_bytes()) + 1,
        }
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
        dst: Value,
        src: Value,
    },
    LoadConst {
        dst: Value,
        value: Const,
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
    BitAnd {
        width: Width,
        dst: Value,
        lhs: Value,
        rhs: Value,
    },
    Dec {
        width: Width,
        value: Value,
    },
    Inc {
        width: Width,
        value: Value,
    },
    Compare {
        width: Width,
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
    Push {
        width: Width,
        value: Value,
    },
    CallProcedure {
        name: String,
    },
    CallFunction {
        name: String,
        dst: Value,
    },
    CleanupStack {
        bytes: u16,
    },
    Return,
    ReturnValue(Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
    Long,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    Symbol(String),
    Direct(String),
    SymbolIndexedX(String),
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
