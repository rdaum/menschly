use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub globals: Vec<Global>,
    pub vectors: Vec<Vector>,
    pub metadata: Vec<Metadata>,
    pub routines: Vec<Routine>,
}

impl Module {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            globals: Vec::new(),
            vectors: Vec::new(),
            metadata: Vec::new(),
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
pub struct Routine {
    pub name: String,
    pub kind: RoutineKind,
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
            abi: Abi::Internal,
            params: Vec::new(),
            locals: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn exported(mut self) -> Self {
        self.abi = Abi::Exported;
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
    CallProcedure {
        name: String,
    },
    CallFunction {
        name: String,
        dst: Value,
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
