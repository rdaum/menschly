use std::collections::BTreeMap;
use std::fmt;

use crate::ir::{
    Abi, Address, BinaryOp, Const, CopRegister, DataItem, Entry, EntryKind, Field, Global,
    GlobalPlacement, Import, InterruptPreserve, IrInst, Label, Library, Local, Metadata, Module,
    Param, PointerKind, Routine, RoutineKind, RoutineType, StaticData, Storage, Type, Value,
    Vector, Width,
};

pub fn parse_source(source: &str) -> Result<SourceModule, FrontendError> {
    Parser::new(lex(source)?)?.parse_module()
}

pub fn compile_source(source: &str) -> Result<Module, FrontendError> {
    lower_module(parse_source(source)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceModule {
    pub name: String,
    pub imports: Vec<ImportDecl>,
    pub libraries: Vec<LibraryDecl>,
    pub entry: Option<EntryDecl>,
    pub types: Vec<TypeDecl>,
    pub consts: Vec<ConstDecl>,
    pub globals: Vec<GlobalDecl>,
    pub vectors: Vec<VectorDecl>,
    pub metadata: Vec<MetadataDecl>,
    pub externals: Vec<ExternalDecl>,
    pub routines: Vec<RoutineDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDecl {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub module: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryDecl {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryDecl {
    pub kind: EntryKind,
    pub routine: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstDecl {
    pub name: String,
    pub value: i64,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalDecl {
    pub name: String,
    pub ty: Type,
    pub placement: GlobalPlacement,
    pub volatile: bool,
    pub export: bool,
    pub arena: bool,
    /// Initial contents (`var t: T = ...`, or a `const` table).
    pub init: Option<Init>,
    /// A `const` table: reads only.
    pub readonly: bool,
}

/// An initialiser: an expression, or the elements of an array (`[...]`) or
/// the fields of a record (`(...)`), in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Init {
    Expr(Expr),
    List(Vec<Init>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorDecl {
    pub name: String,
    pub routine: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataDecl {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDecl {
    pub name: String,
    pub kind: RoutineDeclKind,
    pub abi: Abi,
    pub interrupt_preserve: InterruptPreserve,
    pub interrupt_raw: bool,
    pub interrupt_cop: bool,
    pub params: Vec<VarDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineDecl {
    pub name: String,
    pub kind: RoutineDeclKind,
    pub abi: Abi,
    pub interrupt_preserve: InterruptPreserve,
    pub interrupt_raw: bool,
    pub interrupt_cop: bool,
    pub export: bool,
    pub params: Vec<VarDecl>,
    pub locals: Vec<VarDecl>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutineDeclKind {
    Procedure,
    Function { result: Type },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarDecl {
    pub name: String,
    pub ty: Type,
    pub storage: Storage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Assign {
        target: Expr,
        value: Expr,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    If {
        cond: Cond,
        body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While {
        cond: Cond,
        body: Vec<Stmt>,
    },
    Repeat {
        body: Vec<Stmt>,
        until: Cond,
    },
    /// `for var = start to end` (or `downto`): the bounds are inclusive,
    /// and `end` is evaluated on every pass.
    For {
        var: String,
        start: Expr,
        end: Expr,
        down: bool,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    InlineAsm(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cond {
    Compare(CondCompare),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    Not(Box<Cond>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CondCompare {
    pub lhs: Expr,
    pub op: CondOp,
    pub rhs: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Number(u32),
    String(String),
    Name(String),
    Call {
        name: String,
        args: Vec<Expr>,
    },
    ResultOk(Box<Expr>),
    ResultErr(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Rem(Box<Expr>, Box<Expr>),
    /// Unary minus.
    Neg(Box<Expr>),
    BitAnd(Box<Expr>, Box<Expr>),
    BitOr(Box<Expr>, Box<Expr>),
    BitXor(Box<Expr>, Box<Expr>),
    ShiftLeft(Box<Expr>, Box<Expr>),
    ShiftRight(Box<Expr>, Box<Expr>),
    Deref(Box<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontendError {
    Lex(String),
    Parse(String),
    Type(String),
}

impl fmt::Display for FrontendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(message) => write!(f, "lex error: {message}"),
            Self::Parse(message) => write!(f, "parse error: {message}"),
            Self::Type(message) => write!(f, "type error: {message}"),
        }
    }
}

impl std::error::Error for FrontendError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: TokenKind,
    offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Ident(String),
    Number(u32),
    String(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Dot,
    Caret,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    ShiftLeft,
    ShiftRight,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    EqualEqual,
    BangEqual,
    Eof,
}

fn lex(source: &str) -> Result<Vec<Token>, FrontendError> {
    let mut out = Vec::new();
    let mut chars = source.char_indices().peekable();

    while let Some((offset, ch)) = chars.next() {
        match ch {
            ' ' | '\t' | '\r' | '\n' => {}
            '"' => out.push(token(
                TokenKind::String(read_string(&mut chars, offset)?),
                offset,
            )),
            '\'' => out.push(token(
                TokenKind::Number(read_char_literal(&mut chars, offset)?),
                offset,
            )),
            '#' => {
                while let Some((_, next)) = chars.peek() {
                    if *next == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '(' => out.push(token(TokenKind::LParen, offset)),
            ')' => out.push(token(TokenKind::RParen, offset)),
            '[' => out.push(token(TokenKind::LBracket, offset)),
            ']' => out.push(token(TokenKind::RBracket, offset)),
            ':' => out.push(token(TokenKind::Colon, offset)),
            ',' => out.push(token(TokenKind::Comma, offset)),
            '.' => out.push(token(TokenKind::Dot, offset)),
            '^' => out.push(token(TokenKind::Caret, offset)),
            '+' => out.push(token(TokenKind::Plus, offset)),
            '-' => out.push(token(TokenKind::Minus, offset)),
            '*' => out.push(token(TokenKind::Star, offset)),
            '/' => out.push(token(TokenKind::Slash, offset)),
            '%' => out.push(token(TokenKind::Percent, offset)),
            '&' => out.push(token(TokenKind::Amp, offset)),
            '|' => out.push(token(TokenKind::Pipe, offset)),
            '<' => {
                if matches!(chars.peek(), Some((_, '<'))) {
                    chars.next();
                    out.push(token(TokenKind::ShiftLeft, offset));
                } else if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    out.push(token(TokenKind::LessEqual, offset));
                } else {
                    out.push(token(TokenKind::Less, offset));
                }
            }
            '>' => {
                if matches!(chars.peek(), Some((_, '>'))) {
                    chars.next();
                    out.push(token(TokenKind::ShiftRight, offset));
                } else if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    out.push(token(TokenKind::GreaterEqual, offset));
                } else {
                    out.push(token(TokenKind::Greater, offset));
                }
            }
            '=' => {
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    out.push(token(TokenKind::EqualEqual, offset));
                } else {
                    out.push(token(TokenKind::Equal, offset));
                }
            }
            '!' => {
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    out.push(token(TokenKind::BangEqual, offset));
                } else {
                    return Err(FrontendError::Lex(format!("unexpected '!' at {offset}")));
                }
            }
            '$' => out.push(token(
                TokenKind::Number(read_number(&mut chars, 16, offset)?),
                offset,
            )),
            '0' => {
                if matches!(chars.peek(), Some((_, 'x' | 'X'))) {
                    chars.next();
                    out.push(token(
                        TokenKind::Number(read_number(&mut chars, 16, offset)?),
                        offset,
                    ));
                } else {
                    out.push(token(
                        TokenKind::Number(read_number_with_first(&mut chars, 10, offset, ch)?),
                        offset,
                    ));
                }
            }
            '1'..='9' => out.push(token(
                TokenKind::Number(read_number_with_first(&mut chars, 10, offset, ch)?),
                offset,
            )),
            '_' | 'a'..='z' | 'A'..='Z' => {
                let mut ident = String::from(ch);
                while let Some((_, next)) = chars.peek() {
                    if next.is_ascii_alphanumeric() || *next == '_' {
                        ident.push(*next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(token(TokenKind::Ident(ident), offset));
            }
            other => {
                return Err(FrontendError::Lex(format!(
                    "unexpected character '{other}' at {offset}"
                )));
            }
        }
    }

    out.push(token(TokenKind::Eof, source.len()));
    Ok(out)
}

fn token(kind: TokenKind, offset: usize) -> Token {
    Token { kind, offset }
}

fn read_string(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    offset: usize,
) -> Result<String, FrontendError> {
    let mut out = String::new();
    while let Some((_, ch)) = chars.next() {
        match ch {
            '"' => return Ok(out),
            '\\' => {
                let Some((_, escaped)) = chars.next() else {
                    return Err(FrontendError::Lex(format!(
                        "unterminated escape at {offset}"
                    )));
                };
                match escaped {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'x' => out.push(read_ascii_hex_escape(chars, offset)? as char),
                    other => {
                        return Err(FrontendError::Lex(format!(
                            "unsupported escape \\{other} at {offset}"
                        )));
                    }
                }
            }
            other => out.push(other),
        }
    }
    Err(FrontendError::Lex(format!(
        "unterminated string at {offset}"
    )))
}

fn read_char_literal(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    offset: usize,
) -> Result<u32, FrontendError> {
    let Some((_, ch)) = chars.next() else {
        return Err(FrontendError::Lex(format!(
            "unterminated character literal at {offset}"
        )));
    };
    let value = match ch {
        '\'' => {
            return Err(FrontendError::Lex(format!(
                "empty character literal at {offset}"
            )));
        }
        '\\' => {
            let Some((_, escaped)) = chars.next() else {
                return Err(FrontendError::Lex(format!(
                    "unterminated escape at {offset}"
                )));
            };
            match escaped {
                'n' => b'\n',
                'r' => b'\r',
                't' => b'\t',
                '\'' => b'\'',
                '\\' => b'\\',
                '0' => 0,
                'x' => read_ascii_hex_escape(chars, offset)?,
                other => {
                    return Err(FrontendError::Lex(format!(
                        "unsupported escape \\{other} at {offset}"
                    )));
                }
            }
        }
        other if other.is_ascii() => other as u8,
        other => {
            return Err(FrontendError::Lex(format!(
                "non-ASCII character literal '{other}' at {offset}"
            )));
        }
    };
    match chars.next() {
        Some((_, '\'')) => Ok(value as u32),
        _ => Err(FrontendError::Lex(format!(
            "unterminated character literal at {offset}"
        ))),
    }
}

fn read_ascii_hex_escape(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    offset: usize,
) -> Result<u8, FrontendError> {
    let Some((_, hi)) = chars.next() else {
        return Err(FrontendError::Lex(format!(
            "unterminated hex escape at {offset}"
        )));
    };
    let Some((_, lo)) = chars.next() else {
        return Err(FrontendError::Lex(format!(
            "unterminated hex escape at {offset}"
        )));
    };
    let Some(hi) = hi.to_digit(16) else {
        return Err(FrontendError::Lex(format!(
            "invalid hex escape at {offset}"
        )));
    };
    let Some(lo) = lo.to_digit(16) else {
        return Err(FrontendError::Lex(format!(
            "invalid hex escape at {offset}"
        )));
    };
    let value = ((hi << 4) | lo) as u8;
    if value > 0x7f {
        return Err(FrontendError::Lex(format!(
            "non-ASCII hex escape at {offset}"
        )));
    }
    Ok(value)
}

fn read_number(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    radix: u32,
    offset: usize,
) -> Result<u32, FrontendError> {
    let mut value = 0u32;
    let mut saw_digit = false;

    while let Some((_, ch)) = chars.peek() {
        let Some(digit) = ch.to_digit(radix) else {
            break;
        };
        saw_digit = true;
        value = value
            .checked_mul(radix)
            .and_then(|value| value.checked_add(digit))
            .ok_or_else(|| FrontendError::Lex(format!("number too large at {offset}")))?;
        chars.next();
    }

    if saw_digit {
        Ok(value)
    } else {
        Err(FrontendError::Lex(format!("expected digits at {offset}")))
    }
}

fn read_number_with_first(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    radix: u32,
    offset: usize,
    first: char,
) -> Result<u32, FrontendError> {
    let first = first
        .to_digit(radix)
        .ok_or_else(|| FrontendError::Lex(format!("invalid number at {offset}")))?;
    let rest = read_number_or_empty(chars, radix, offset)?;
    first
        .checked_mul(radix.pow(rest.1))
        .and_then(|value| value.checked_add(rest.0))
        .ok_or_else(|| FrontendError::Lex(format!("number too large at {offset}")))
}

fn read_number_or_empty(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    radix: u32,
    offset: usize,
) -> Result<(u32, u32), FrontendError> {
    let mut value = 0u32;
    let mut digits = 0u32;

    while let Some((_, ch)) = chars.peek() {
        let Some(digit) = ch.to_digit(radix) else {
            break;
        };
        digits += 1;
        value = value
            .checked_mul(radix)
            .and_then(|value| value.checked_add(digit))
            .ok_or_else(|| FrontendError::Lex(format!("number too large at {offset}")))?;
        chars.next();
    }

    Ok((value, digits))
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    types: BTreeMap<String, Type>,
    /// Scalar constants declared so far, for constant expressions.
    consts: BTreeMap<String, i64>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Result<Self, FrontendError> {
        Ok(Self {
            tokens,
            pos: 0,
            types: BTreeMap::new(),
            consts: BTreeMap::new(),
        })
    }

    fn parse_module(&mut self) -> Result<SourceModule, FrontendError> {
        self.expect_keyword("module")?;
        let name = self.expect_ident()?;
        let mut imports = Vec::new();
        let mut libraries = Vec::new();
        let mut entry = None;
        let mut types = Vec::new();
        let mut consts = Vec::new();
        let mut globals = Vec::new();
        let mut vectors = Vec::new();
        let mut metadata = Vec::new();
        let mut externals = Vec::new();
        let mut routines = Vec::new();

        while !self.at_eof() {
            if self.take_keyword("import") {
                imports.push(self.parse_import_decl()?);
            } else if self.take_keyword("library") {
                libraries.push(self.parse_library_decl()?);
            } else if self.take_keyword("application") {
                entry = Some(EntryDecl {
                    kind: EntryKind::Application,
                    routine: self.expect_ident()?,
                });
            } else if self.take_keyword("command") {
                entry = Some(EntryDecl {
                    kind: EntryKind::Command,
                    routine: self.expect_ident()?,
                });
            } else if self.take_keyword("metadata") {
                metadata.push(self.parse_metadata_decl()?);
            } else if self.take_keyword("vector") {
                vectors.push(self.parse_vector_decl()?);
            } else if self.take_keyword("type") {
                types.push(self.parse_type_decl()?);
            } else if self.take_keyword("const") {
                match self.parse_const_decl()? {
                    ConstOrTable::Const(decl) => consts.push(decl),
                    ConstOrTable::Table(decl) => globals.push(decl),
                }
            } else if self.take_keyword("external") {
                externals.push(self.parse_external_decl()?);
            } else if self.starts_global_decl() {
                globals.push(self.parse_global_decl()?);
            } else {
                routines.push(self.parse_routine()?);
            }
        }

        Ok(SourceModule {
            name,
            imports,
            libraries,
            entry,
            types,
            consts,
            globals,
            vectors,
            metadata,
            externals,
            routines,
        })
    }

    fn parse_import_decl(&mut self) -> Result<ImportDecl, FrontendError> {
        let module = self.expect_ident()?;
        let alias = if self.take_keyword("as") {
            Some(self.expect_ident()?)
        } else {
            None
        };
        Ok(ImportDecl { module, alias })
    }

    fn parse_library_decl(&mut self) -> Result<LibraryDecl, FrontendError> {
        Ok(LibraryDecl {
            name: self.expect_ident()?,
        })
    }

    fn parse_metadata_decl(&mut self) -> Result<MetadataDecl, FrontendError> {
        let key = self.expect_ident()?;
        self.expect(TokenKind::Equal)?;
        let value = if let Some(value) = self.take_string() {
            value
        } else if let Some(value) = self.take_number() {
            value.to_string()
        } else {
            self.expect_ident()?
        };
        Ok(MetadataDecl { key, value })
    }

    fn parse_vector_decl(&mut self) -> Result<VectorDecl, FrontendError> {
        let name = self.expect_ident()?;
        self.expect(TokenKind::Equal)?;
        let routine = self.expect_ident()?;
        Ok(VectorDecl { name, routine })
    }

    fn parse_type_decl(&mut self) -> Result<TypeDecl, FrontendError> {
        let name = self.expect_ident()?;
        self.expect(TokenKind::Equal)?;
        let ty = self.parse_type()?;
        if self.types.insert(name.clone(), ty.clone()).is_some() {
            return Err(FrontendError::Parse(format!("duplicate type {name}")));
        }
        Ok(TypeDecl { name, ty })
    }

    /// `const NAME = expr`, `const NAME: scalar = expr`, or a table:
    /// `const NAME: [N]T = [...]` / `const NAME: Record = (...)`.
    fn parse_const_decl(&mut self) -> Result<ConstOrTable, FrontendError> {
        let name = self.expect_ident()?;
        let declared = if self.take(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(TokenKind::Equal)?;
        if let Some(ty) = declared.as_ref().filter(|ty| integer_width(ty).is_none()) {
            let init = self.parse_init(ty)?;
            return Ok(ConstOrTable::Table(GlobalDecl {
                name,
                ty: ty.clone(),
                placement: GlobalPlacement::Static,
                volatile: false,
                export: false,
                arena: false,
                init: Some(init),
                readonly: true,
            }));
        }
        let expr = self.parse_expr()?;
        let consts = &self.consts;
        let value = const_eval(&expr, &|name| consts.get(name).copied())
            .map_err(|error| FrontendError::Parse(format!("constant {name}: {error}")))?;
        if !(-0x8000_0000..=0xffff_ffff).contains(&value) {
            return Err(FrontendError::Parse(format!(
                "constant {name} = {value} does not fit in 32 bits"
            )));
        }
        let ty = match declared {
            Some(ty) => {
                const_for_type(value, &ty)?;
                ty
            }
            None => const_type(value),
        };
        self.consts.insert(name.clone(), value);
        Ok(ConstOrTable::Const(ConstDecl { name, value, ty }))
    }

    /// An initialiser for a value of type `ty`.
    fn parse_init(&mut self, ty: &Type) -> Result<Init, FrontendError> {
        let (close, elements): (TokenKind, Vec<Type>) = match ty {
            Type::Array { len, element } if self.take(TokenKind::LBracket) => (
                TokenKind::RBracket,
                vec![(**element).clone(); usize::from(*len)],
            ),
            Type::Record { fields } if self.take(TokenKind::LParen) => (
                TokenKind::RParen,
                fields.iter().map(|field| field.ty.clone()).collect(),
            ),
            _ => return Ok(Init::Expr(self.parse_expr()?)),
        };
        let mut items = Vec::new();
        while !self.take(close.clone()) {
            let element = elements.get(items.len()).ok_or_else(|| {
                self.parse_error(&format!(
                    "too many initialisers (expected {})",
                    elements.len()
                ))
            })?;
            items.push(self.parse_init(&element.clone())?);
            if !self.take(TokenKind::Comma) {
                self.expect(close.clone())?;
                break;
            }
        }
        Ok(Init::List(items))
    }

    fn parse_external_decl(&mut self) -> Result<ExternalDecl, FrontendError> {
        let header = self.parse_routine_header()?;
        Ok(ExternalDecl {
            name: header.name,
            kind: header.kind,
            abi: header.abi,
            interrupt_preserve: header.interrupt.preserve,
            interrupt_raw: header.interrupt.raw,
            interrupt_cop: header.interrupt.cop,
            params: header.params,
        })
    }

    fn parse_global_decl(&mut self) -> Result<GlobalDecl, FrontendError> {
        let export = self.take_keyword("export");
        let arena = self.take_keyword("arena");
        let placement = self.parse_global_placement()?;
        let volatile = self.take_keyword("volatile");
        self.expect_keyword("var")?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        let init = if self.take(TokenKind::Equal) {
            if !matches!(placement, GlobalPlacement::Static) || volatile {
                return Err(self.parse_error("only plain variables can be initialised"));
            }
            Some(self.parse_init(&ty)?)
        } else {
            None
        };
        Ok(GlobalDecl {
            name,
            ty,
            placement,
            volatile,
            export: export || arena,
            arena,
            init,
            readonly: false,
        })
    }

    fn parse_global_placement(&mut self) -> Result<GlobalPlacement, FrontendError> {
        if self.take_keyword("fixed") {
            self.expect(TokenKind::LParen)?;
            let address = self
                .take_number()
                .ok_or_else(|| self.parse_error("expected fixed address"))?;
            self.expect(TokenKind::RParen)?;
            Ok(GlobalPlacement::Fixed(address))
        } else if self.take_keyword("bank") {
            self.expect(TokenKind::LParen)?;
            let bank = self
                .take_number()
                .ok_or_else(|| self.parse_error("expected bank number"))?;
            self.expect(TokenKind::RParen)?;
            if bank > u8::MAX as u32 {
                Err(FrontendError::Parse(format!("bank {bank} is out of range")))
            } else {
                Ok(GlobalPlacement::Bank(bank as u8))
            }
        } else {
            Ok(GlobalPlacement::Static)
        }
    }

    fn parse_routine(&mut self) -> Result<RoutineDecl, FrontendError> {
        let RoutineHeader {
            export,
            abi,
            interrupt,
            kind,
            name,
            params,
        } = self.parse_routine_header()?;

        let mut locals = Vec::new();
        let mut body = Vec::new();
        while !self.take_keyword("end") {
            if self.at_eof() {
                return Err(self.parse_error("unterminated routine"));
            }
            if self.starts_var_decl() {
                locals.push(self.parse_var_decl()?);
            } else {
                body.push(self.parse_stmt()?);
            }
        }

        Ok(RoutineDecl {
            name,
            kind,
            abi,
            interrupt_preserve: interrupt.preserve,
            interrupt_raw: interrupt.raw,
            interrupt_cop: interrupt.cop,
            export,
            params,
            locals,
            body,
        })
    }

    fn parse_routine_header(&mut self) -> Result<RoutineHeader, FrontendError> {
        let export = self.take_keyword("export");
        let mut abi = Abi::Internal;
        let mut interrupt = InterruptAttr {
            preserve: InterruptPreserve::NONE,
            raw: false,
            cop: false,
        };
        if self.take_keyword("far") {
            abi = Abi::Far;
        } else if self.take_keyword("interrupt") {
            abi = Abi::Interrupt;
            interrupt = self.parse_interrupt_attr()?;
        } else if self.take_keyword("reentrant") {
            abi = Abi::Reentrant;
        }

        let is_function = if self.take_keyword("procedure") {
            false
        } else if self.take_keyword("function") {
            true
        } else {
            return Err(self.parse_error("expected procedure or function"));
        };

        let name = self.expect_ident()?;
        let params = self.parse_params()?;
        let kind = if is_function {
            self.expect(TokenKind::Colon)?;
            RoutineDeclKind::Function {
                result: self.parse_type()?,
            }
        } else {
            RoutineDeclKind::Procedure
        };

        Ok(RoutineHeader {
            export,
            abi,
            interrupt,
            kind,
            name,
            params,
        })
    }

    fn parse_interrupt_attr(&mut self) -> Result<InterruptAttr, FrontendError> {
        if !self.take(TokenKind::LParen) {
            return Ok(InterruptAttr::default());
        }

        let mut cop = false;
        if self.take_keyword("cop") {
            cop = true;
            if self.take(TokenKind::RParen) {
                return Ok(InterruptAttr {
                    preserve: InterruptPreserve::DEFAULT,
                    raw: false,
                    cop,
                });
            }
            self.expect(TokenKind::Comma)?;
        }

        if self.take_keyword("raw") {
            self.expect(TokenKind::RParen)?;
            return Ok(InterruptAttr {
                preserve: InterruptPreserve::NONE,
                raw: true,
                cop,
            });
        }

        self.expect_keyword("preserve")?;
        self.expect(TokenKind::Equal)?;
        let mut preserve = InterruptPreserve::NONE;
        let first = self.expect_ident()?;
        match first.as_str() {
            "default" => {
                preserve = InterruptPreserve::DEFAULT;
                self.expect(TokenKind::RParen)?;
                return Ok(InterruptAttr {
                    preserve,
                    raw: false,
                    cop,
                });
            }
            "none" => {
                self.expect(TokenKind::RParen)?;
                return Ok(InterruptAttr {
                    preserve,
                    raw: false,
                    cop,
                });
            }
            _ => apply_interrupt_preserve_name(&mut preserve, &first)?,
        }

        while self.take(TokenKind::Comma) {
            let name = self.expect_ident()?;
            apply_interrupt_preserve_name(&mut preserve, &name)?;
        }
        self.expect(TokenKind::RParen)?;
        Ok(InterruptAttr {
            preserve,
            raw: false,
            cop,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<VarDecl>, FrontendError> {
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if self.take(TokenKind::RParen) {
            return Ok(params);
        }

        loop {
            let storage = self.parse_storage_prefix();
            let name = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            params.push(VarDecl { name, ty, storage });

            if self.take(TokenKind::RParen) {
                break;
            }
            self.expect(TokenKind::Comma)?;
        }

        Ok(params)
    }

    fn parse_var_decl(&mut self) -> Result<VarDecl, FrontendError> {
        let storage = self.parse_storage_prefix();
        self.expect_keyword("var")?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        Ok(VarDecl { name, ty, storage })
    }

    fn parse_storage_prefix(&mut self) -> Storage {
        if self.take_keyword("direct") {
            Storage::Direct
        } else if self.take_keyword("static") {
            Storage::Static
        } else if self.take_keyword("scratch") {
            Storage::Scratch
        } else if self.take_keyword("stack") {
            Storage::Stack
        } else if self.take_keyword("far") {
            Storage::Far
        } else {
            Storage::Default
        }
    }

    fn parse_type(&mut self) -> Result<Type, FrontendError> {
        if self.take(TokenKind::LBracket) {
            if self.take(TokenKind::RBracket) {
                return Ok(Type::Slice {
                    element: Box::new(self.parse_type()?),
                });
            }
            let len = self
                .take_number()
                .ok_or_else(|| self.parse_error("expected array length"))?;
            self.expect(TokenKind::RBracket)?;
            if len > u16::MAX as u32 {
                return Err(FrontendError::Parse(format!(
                    "array length {len} is out of range"
                )));
            }
            return Ok(Type::Array {
                len: len as u16,
                element: Box::new(self.parse_type()?),
            });
        }
        if self.take_keyword("record") {
            return self.parse_record_type();
        }
        if self.take_keyword("result") {
            self.expect(TokenKind::LParen)?;
            let ok = self.parse_type()?;
            self.expect(TokenKind::Comma)?;
            let err = self.parse_type()?;
            self.expect(TokenKind::RParen)?;
            return Ok(Type::Result {
                ok: Box::new(ok),
                err: Box::new(err),
            });
        }
        if self.take_keyword("near") {
            self.expect(TokenKind::Caret)?;
            return Ok(Type::near_ptr(self.parse_type()?));
        }
        if self.take_keyword("far") {
            self.expect(TokenKind::Caret)?;
            return Ok(Type::far_ptr(self.parse_type()?));
        }
        if self.take_keyword("direct") {
            self.expect(TokenKind::Caret)?;
            return Ok(Type::Pointer {
                to: Box::new(self.parse_type()?),
                kind: PointerKind::Direct,
            });
        }
        if self.is_keyword_at(0, "reentrant")
            || self.is_keyword_at(0, "procedure")
            || self.is_keyword_at(0, "function")
        {
            return self.parse_routine_type();
        }

        let name = self.expect_ident()?;
        match name.as_str() {
            "u8" => Ok(Type::U8),
            "i8" => Ok(Type::I8),
            "u16" => Ok(Type::U16),
            "i16" => Ok(Type::I16),
            "u24" => Ok(Type::U24),
            "u32" => Ok(Type::U32),
            "i32" => Ok(Type::I32),
            _ => self
                .types
                .get(&name)
                .cloned()
                .ok_or_else(|| FrontendError::Parse(format!("unknown type {name}"))),
        }
    }

    /// `[reentrant] procedure(params)` or `[reentrant] function(params): T`,
    /// where each parameter is a type, optionally named (`count: u16`).
    fn parse_routine_type(&mut self) -> Result<Type, FrontendError> {
        let abi = if self.take_keyword("reentrant") {
            Abi::Reentrant
        } else {
            Abi::Internal
        };
        let is_function = if self.take_keyword("function") {
            true
        } else {
            self.expect_keyword("procedure")?;
            false
        };
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.take(TokenKind::RParen) {
            loop {
                if matches!(
                    self.tokens.get(self.pos + 1).map(|token| &token.kind),
                    Some(TokenKind::Colon)
                ) {
                    self.expect_ident()?;
                    self.expect(TokenKind::Colon)?;
                }
                params.push(self.parse_type()?);
                if self.take(TokenKind::RParen) {
                    break;
                }
                self.expect(TokenKind::Comma)?;
            }
        }
        let result = if is_function {
            self.expect(TokenKind::Colon)?;
            Some(self.parse_type()?)
        } else {
            None
        };
        let routine = RoutineType {
            // Without parameters the two conventions are the same call.
            abi: if params.is_empty() {
                Abi::Internal
            } else {
                abi
            },
            params,
            result,
        };
        if !routine.params.is_empty() && !matches!(routine.abi, Abi::Reentrant) {
            return Err(self.parse_error(
                "routine types with parameters must be reentrant (arguments on the stack)",
            ));
        }
        Ok(Type::Routine(Box::new(routine)))
    }

    fn parse_record_type(&mut self) -> Result<Type, FrontendError> {
        let end = if self.take(TokenKind::LParen) {
            TokenKind::RParen
        } else {
            TokenKind::Eof
        };
        let mut fields = Vec::new();
        let mut offset = 0u16;
        if matches!(end, TokenKind::RParen) && self.take(TokenKind::RParen) {
            return Ok(Type::Record { fields });
        }
        loop {
            if matches!(end, TokenKind::Eof) && self.take_keyword("end") {
                break;
            }
            let name = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            if fields.iter().any(|field: &Field| field.name == name) {
                return Err(FrontendError::Parse(format!(
                    "duplicate record field {name}"
                )));
            }
            fields.push(Field {
                name,
                offset,
                ty: ty.clone(),
            });
            offset = offset
                .checked_add(ty.size_bytes())
                .ok_or_else(|| FrontendError::Parse("record is too large".to_owned()))?;
            if matches!(end, TokenKind::RParen) {
                if self.take(TokenKind::RParen) {
                    break;
                }
                self.expect(TokenKind::Comma)?;
            } else {
                self.take(TokenKind::Comma);
            }
        }
        Ok(Type::Record { fields })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, FrontendError> {
        if self.take_keyword("if") {
            return self.parse_if_after_keyword();
        }
        if self.take_keyword("while") {
            let cond = self.parse_cond()?;
            let body = self.parse_block()?;
            return Ok(Stmt::While { cond, body });
        }
        if self.take_keyword("for") {
            let var = self.expect_ident()?;
            self.expect(TokenKind::Equal)?;
            let start = self.parse_expr()?;
            let down = if self.take_keyword("downto") {
                true
            } else {
                self.expect_keyword("to")?;
                false
            };
            let end = self.parse_expr()?;
            let body = self.parse_block()?;
            return Ok(Stmt::For {
                var,
                start,
                end,
                down,
                body,
            });
        }
        if self.take_keyword("repeat") {
            let (body, _terminator) = self.parse_block_until(&["until"])?;
            let until = self.parse_cond()?;
            return Ok(Stmt::Repeat { body, until });
        }
        if self.take_keyword("return") {
            let value = if self.next_is_block_end() || self.at_eof() {
                None
            } else {
                Some(self.parse_expr()?)
            };
            return Ok(Stmt::Return(value));
        }
        if self.take_keyword("asm") {
            let Some(line) = self.take_string() else {
                return Err(self.parse_error("expected inline assembly string"));
            };
            return Ok(Stmt::InlineAsm(line));
        }

        let expr = self.parse_expr()?;
        if self.take(TokenKind::Equal) {
            return Ok(Stmt::Assign {
                target: expr,
                value: self.parse_expr()?,
            });
        }

        if let Expr::Call { name, args } = expr {
            Ok(Stmt::Call { name, args })
        } else {
            Err(self.parse_error("expected assignment or call statement"))
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, FrontendError> {
        let (body, _terminator) = self.parse_block_until(&["end"])?;
        Ok(body)
    }

    fn parse_if_after_keyword(&mut self) -> Result<Stmt, FrontendError> {
        let cond = self.parse_cond()?;
        let (body, terminator) = self.parse_block_until(&["elsif", "else", "end"])?;
        let else_body = match terminator.as_str() {
            "elsif" => vec![self.parse_if_after_keyword()?],
            "else" => self.parse_block()?,
            "end" => Vec::new(),
            _ => unreachable!("if parser returned unexpected terminator"),
        };
        Ok(Stmt::If {
            cond,
            body,
            else_body,
        })
    }

    fn parse_block_until(
        &mut self,
        terminators: &[&str],
    ) -> Result<(Vec<Stmt>, String), FrontendError> {
        let mut body = Vec::new();
        loop {
            for terminator in terminators {
                if self.take_keyword(terminator) {
                    return Ok((body, (*terminator).to_owned()));
                }
            }
            if self.at_eof() {
                return Err(self.parse_error("unterminated block"));
            }
            body.push(self.parse_stmt()?);
        }
    }

    fn parse_cond(&mut self) -> Result<Cond, FrontendError> {
        self.parse_cond_or()
    }

    fn parse_cond_or(&mut self) -> Result<Cond, FrontendError> {
        let mut cond = self.parse_cond_and()?;
        while self.take_keyword("or") {
            let rhs = self.parse_cond_and()?;
            cond = Cond::Or(Box::new(cond), Box::new(rhs));
        }
        Ok(cond)
    }

    fn parse_cond_and(&mut self) -> Result<Cond, FrontendError> {
        let mut cond = self.parse_cond_unary()?;
        while self.take_keyword("and") {
            let rhs = self.parse_cond_unary()?;
            cond = Cond::And(Box::new(cond), Box::new(rhs));
        }
        Ok(cond)
    }

    /// `not c`, a parenthesised condition, or a comparison.
    fn parse_cond_unary(&mut self) -> Result<Cond, FrontendError> {
        if self.take_keyword("not") {
            return Ok(Cond::Not(Box::new(self.parse_cond_unary()?)));
        }
        if matches!(self.peek(), Some(TokenKind::LParen)) {
            // `(a + b) == c` starts like a grouped condition: back off if
            // what follows the parentheses continues an expression.
            let start = self.pos;
            self.pos += 1;
            if let Ok(cond) = self.parse_cond() {
                if self.take(TokenKind::RParen) && !self.continues_expression() {
                    return Ok(cond);
                }
            }
            self.pos = start;
        }
        self.parse_cond_compare()
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|token| &token.kind)
    }

    /// Whether the next token extends an expression or compares it.
    fn continues_expression(&self) -> bool {
        matches!(
            self.peek(),
            Some(
                TokenKind::EqualEqual
                    | TokenKind::BangEqual
                    | TokenKind::Less
                    | TokenKind::LessEqual
                    | TokenKind::Greater
                    | TokenKind::GreaterEqual
                    | TokenKind::Plus
                    | TokenKind::Minus
                    | TokenKind::Star
                    | TokenKind::Slash
                    | TokenKind::Percent
                    | TokenKind::Amp
                    | TokenKind::Pipe
                    | TokenKind::ShiftLeft
                    | TokenKind::ShiftRight
                    | TokenKind::Caret
                    | TokenKind::LBracket
                    | TokenKind::Dot
            )
        ) || self.is_keyword_at(0, "xor")
    }

    fn parse_cond_compare(&mut self) -> Result<Cond, FrontendError> {
        let lhs = self.parse_expr()?;
        let op = if self.take(TokenKind::EqualEqual) {
            CondOp::Eq
        } else if self.take(TokenKind::BangEqual) {
            CondOp::Ne
        } else if self.take(TokenKind::LessEqual) {
            CondOp::Le
        } else if self.take(TokenKind::Less) {
            CondOp::Lt
        } else if self.take(TokenKind::GreaterEqual) {
            CondOp::Ge
        } else if self.take(TokenKind::Greater) {
            CondOp::Gt
        } else {
            // A bare value is true when it is not zero.
            return Ok(Cond::Compare(CondCompare {
                lhs,
                op: CondOp::Ne,
                rhs: Expr::Number(0),
            }));
        };
        let rhs = self.parse_expr()?;
        Ok(Cond::Compare(CondCompare { lhs, op, rhs }))
    }

    /// Binary operator precedence follows C: `|` < `xor` < `&` < shifts <
    /// `+`/`-` < `*`/`/`/`%` < unary `-`.
    fn parse_expr(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_bit_xor()?;
        while self.take(TokenKind::Pipe) {
            let rhs = self.parse_bit_xor()?;
            expr = Expr::BitOr(Box::new(expr), Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_bit_xor(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_bit_and()?;
        while self.take_keyword("xor") {
            let rhs = self.parse_bit_and()?;
            expr = Expr::BitXor(Box::new(expr), Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_bit_and(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_shift()?;
        while self.take(TokenKind::Amp) {
            let rhs = self.parse_shift()?;
            expr = Expr::BitAnd(Box::new(expr), Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_shift(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_add()?;
        loop {
            if self.take(TokenKind::ShiftLeft) {
                let rhs = self.parse_add()?;
                expr = Expr::ShiftLeft(Box::new(expr), Box::new(rhs));
            } else if self.take(TokenKind::ShiftRight) {
                let rhs = self.parse_add()?;
                expr = Expr::ShiftRight(Box::new(expr), Box::new(rhs));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_add(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_mul()?;
        loop {
            if self.take(TokenKind::Plus) {
                let rhs = self.parse_mul()?;
                expr = Expr::Add(Box::new(expr), Box::new(rhs));
            } else if self.take(TokenKind::Minus) {
                let rhs = self.parse_mul()?;
                expr = Expr::Sub(Box::new(expr), Box::new(rhs));
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_mul(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_unary()?;
        loop {
            let make: fn(Box<Expr>, Box<Expr>) -> Expr = if self.take(TokenKind::Star) {
                Expr::Mul
            } else if self.take(TokenKind::Slash) {
                Expr::Div
            } else if self.take(TokenKind::Percent) {
                Expr::Rem
            } else {
                break;
            };
            let rhs = self.parse_unary()?;
            expr = make(Box::new(expr), Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, FrontendError> {
        if self.take(TokenKind::Minus) {
            return Ok(Expr::Neg(Box::new(self.parse_unary()?)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.take(TokenKind::Caret) {
                expr = Expr::Deref(Box::new(expr));
            } else if self.take(TokenKind::LBracket) {
                let index = self.parse_expr()?;
                self.expect(TokenKind::RBracket)?;
                expr = Expr::Index(Box::new(expr), Box::new(index));
            } else if self.take(TokenKind::Dot) {
                let field = self.expect_ident()?;
                expr = Expr::Field(Box::new(expr), field);
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, FrontendError> {
        if self.take(TokenKind::LParen) {
            let expr = self.parse_expr()?;
            self.expect(TokenKind::RParen)?;
            return Ok(expr);
        }
        if let Some(value) = self.take_number() {
            return Ok(Expr::Number(value));
        }
        if let Some(value) = self.take_string() {
            return Ok(Expr::String(value));
        }

        let name = self.expect_ident()?;
        // `size(T)`: the size of a type in bytes, as a constant.
        if name == "size" && self.take(TokenKind::LParen) {
            let ty = self.parse_type()?;
            self.expect(TokenKind::RParen)?;
            return Ok(Expr::Number(u32::from(ty.size_bytes())));
        }
        if self.take(TokenKind::LParen) {
            let mut args = Vec::new();
            if self.take(TokenKind::RParen) {
                if name == "ok" {
                    return Err(self.parse_error("ok needs a value"));
                }
                if name == "err" {
                    return Err(self.parse_error("err needs a value"));
                }
                return Ok(Expr::Call { name, args });
            }
            loop {
                args.push(self.parse_expr()?);
                if self.take(TokenKind::RParen) {
                    break;
                }
                self.expect(TokenKind::Comma)?;
            }
            if name == "ok" || name == "err" {
                if args.len() != 1 {
                    return Err(self.parse_error("result constructor needs one value"));
                }
                let value = Box::new(args.remove(0));
                if name == "ok" {
                    Ok(Expr::ResultOk(value))
                } else {
                    Ok(Expr::ResultErr(value))
                }
            } else {
                Ok(Expr::Call { name, args })
            }
        } else {
            Ok(Expr::Name(name))
        }
    }

    fn starts_var_decl(&self) -> bool {
        self.is_keyword_at(0, "var")
            || ((self.is_keyword_at(0, "direct")
                || self.is_keyword_at(0, "static")
                || self.is_keyword_at(0, "scratch")
                || self.is_keyword_at(0, "stack")
                || self.is_keyword_at(0, "far"))
                && self.is_keyword_at(1, "var"))
    }

    fn starts_global_decl(&self) -> bool {
        self.is_keyword_at(0, "var")
            || self.is_keyword_at(0, "volatile")
            || self.is_keyword_at(0, "arena")
            || self.is_keyword_at(0, "fixed")
            || self.is_keyword_at(0, "bank")
            || (self.is_keyword_at(0, "export")
                && (self.is_keyword_at(1, "var")
                    || self.is_keyword_at(1, "volatile")
                    || self.is_keyword_at(1, "arena")
                    || self.is_keyword_at(1, "fixed")
                    || self.is_keyword_at(1, "bank")))
    }

    fn next_is_block_end(&self) -> bool {
        self.is_keyword_at(0, "end")
            || self.is_keyword_at(0, "else")
            || self.is_keyword_at(0, "elsif")
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), FrontendError> {
        if self.take_keyword(keyword) {
            Ok(())
        } else {
            Err(self.parse_error(&format!("expected {keyword}")))
        }
    }

    fn take_keyword(&mut self, keyword: &str) -> bool {
        if self.is_keyword_at(0, keyword) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn is_keyword_at(&self, offset: usize, keyword: &str) -> bool {
        matches!(
            self.tokens.get(self.pos + offset).map(|token| &token.kind),
            Some(TokenKind::Ident(ident)) if ident == keyword
        )
    }

    fn expect_ident(&mut self) -> Result<String, FrontendError> {
        match self.tokens.get(self.pos).map(|token| &token.kind) {
            Some(TokenKind::Ident(ident)) => {
                self.pos += 1;
                Ok(ident.clone())
            }
            _ => Err(self.parse_error("expected identifier")),
        }
    }

    fn take_number(&mut self) -> Option<u32> {
        match self.tokens.get(self.pos).map(|token| &token.kind) {
            Some(TokenKind::Number(value)) => {
                self.pos += 1;
                Some(*value)
            }
            _ => None,
        }
    }

    fn take_string(&mut self) -> Option<String> {
        match self.tokens.get(self.pos).map(|token| &token.kind) {
            Some(TokenKind::String(value)) => {
                self.pos += 1;
                Some(value.clone())
            }
            _ => None,
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<(), FrontendError> {
        if self.take(kind.clone()) {
            Ok(())
        } else {
            Err(self.parse_error(&format!("expected {kind:?}")))
        }
    }

    fn take(&mut self, kind: TokenKind) -> bool {
        if self.tokens.get(self.pos).map(|token| &token.kind) == Some(&kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn at_eof(&self) -> bool {
        matches!(
            self.tokens.get(self.pos).map(|token| &token.kind),
            Some(TokenKind::Eof)
        )
    }

    fn parse_error(&self, message: &str) -> FrontendError {
        let offset = self
            .tokens
            .get(self.pos)
            .map(|token| token.offset)
            .unwrap_or_default();
        FrontendError::Parse(format!("{message} at {offset}"))
    }
}

fn apply_interrupt_preserve_name(
    preserve: &mut InterruptPreserve,
    name: &str,
) -> Result<(), FrontendError> {
    match name {
        "a" => preserve.a = true,
        "x" => preserve.x = true,
        "y" => preserve.y = true,
        "p" => preserve.p = true,
        "dbr" => preserve.dbr = true,
        other => {
            return Err(FrontendError::Parse(format!(
                "unknown interrupt preserve register {other}"
            )));
        }
    }
    Ok(())
}

fn cop_register_builtin(name: &str) -> Option<CopRegister> {
    match name {
        "cop_a" => Some(CopRegister::A),
        "cop_x" => Some(CopRegister::X),
        "cop_y" => Some(CopRegister::Y),
        _ => None,
    }
}

fn integer_cast_builtin(name: &str) -> Option<Type> {
    match name {
        "u8" => Some(Type::U8),
        "i8" => Some(Type::I8),
        "u16" => Some(Type::U16),
        "i16" => Some(Type::I16),
        "u24" => Some(Type::U24),
        "u32" => Some(Type::U32),
        "i32" => Some(Type::I32),
        _ => None,
    }
}

/// Everything before a routine's body.
struct RoutineHeader {
    export: bool,
    abi: Abi,
    interrupt: InterruptAttr,
    kind: RoutineDeclKind,
    name: String,
    params: Vec<VarDecl>,
}

enum ConstOrTable {
    Const(ConstDecl),
    Table(GlobalDecl),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InterruptAttr {
    preserve: InterruptPreserve,
    raw: bool,
    cop: bool,
}

impl Default for InterruptAttr {
    fn default() -> Self {
        Self {
            preserve: InterruptPreserve::DEFAULT,
            raw: false,
            cop: false,
        }
    }
}

#[derive(Debug, Clone)]
struct RoutineSig {
    kind: RoutineDeclKind,
    abi: Abi,
    params: Vec<ParamSig>,
}

#[derive(Debug, Clone)]
struct ParamSig {
    name: String,
    ty: Type,
}

#[derive(Debug, Clone)]
struct Binding {
    ty: Type,
    value: Value,
    /// A `const` table.
    readonly: bool,
}

struct Lowerer {
    routines: BTreeMap<String, RoutineSig>,
    labels: usize,
    temps: usize,
    strings: Vec<StaticData>,
}

struct RoutineLowerer<'a> {
    parent: &'a mut Lowerer,
    routine: &'a RoutineDecl,
    bindings: BTreeMap<String, Binding>,
    body: Vec<IrInst>,
}

fn lower_module(source: SourceModule) -> Result<Module, FrontendError> {
    let mut lowerer = Lowerer {
        routines: routine_sigs(&source)?,
        labels: 0,
        temps: 0,
        strings: Vec::new(),
    };

    let mut module = Module::new(source.name);
    module.imports = source
        .imports
        .iter()
        .map(|import| Import {
            module: import.module.clone(),
            alias: import.alias.clone(),
        })
        .collect();
    module.libraries = source
        .libraries
        .iter()
        .map(|library| Library {
            name: library.name.clone(),
        })
        .collect();
    module.external_routines = source
        .externals
        .iter()
        .map(|external| external.name.clone())
        .collect();
    module.external_direct_symbols = source
        .externals
        .iter()
        .filter(|external| !matches!(external.abi, Abi::Reentrant))
        .flat_map(|external| {
            external
                .params
                .iter()
                .map(|param| crate::ir::param_symbol(&external.name, &param.name))
        })
        .collect();
    module.entry = source.entry.as_ref().map(|entry| Entry {
        kind: entry.kind,
        routine: entry.routine.clone(),
    });
    for global in &source.globals {
        check_symbol_name("global", &global.name)?;
    }
    for routine in &source.routines {
        check_symbol_name("routine", &routine.name)?;
    }
    let consts: BTreeMap<String, i64> = source
        .consts
        .iter()
        .map(|decl| (decl.name.clone(), decl.value))
        .collect();
    let global_names: Vec<&str> = source
        .globals
        .iter()
        .map(|global| global.name.as_str())
        .collect();
    let mut globals = Vec::new();
    for global in &source.globals {
        let init = match &global.init {
            Some(init) => {
                let mut items = Vec::new();
                lowerer
                    .encode_init(&global.ty, init, &consts, &global_names, &mut items)
                    .map_err(|error| {
                        FrontendError::Type(format!("initialiser of {}: {error}", global.name))
                    })?;
                Some(items)
            }
            None => None,
        };
        globals.push(Global {
            name: global.name.clone(),
            ty: global.ty.clone(),
            placement: global.placement.clone(),
            volatile: global.volatile,
            export: global.export,
            init,
        });
    }
    module.globals = globals;
    module.vectors = source
        .vectors
        .iter()
        .map(|vector| Vector {
            name: vector.name.clone(),
            routine: vector.routine.clone(),
        })
        .collect();
    module.metadata = source
        .metadata
        .iter()
        .map(|metadata| Metadata {
            key: metadata.key.clone(),
            value: metadata.value.clone(),
        })
        .collect();
    for global in &source.globals {
        if global.arena {
            module.metadata.push(Metadata {
                key: "arena".to_owned(),
                value: format!("{}:{}", global.name, global.ty.size_bytes()),
            });
        }
    }

    for routine in &source.routines {
        module.routines.push(lowerer.lower_routine(
            routine,
            &source.globals,
            &source.consts,
            source.entry.as_ref(),
        )?);
    }
    lowerer.validate_vectors_and_entry(&module)?;
    module.static_data = lowerer.strings;
    Ok(module)
}

fn routine_sigs(source: &SourceModule) -> Result<BTreeMap<String, RoutineSig>, FrontendError> {
    let mut out = BTreeMap::new();
    for external in &source.externals {
        insert_routine_sig(
            &mut out,
            &external.name,
            RoutineSig {
                kind: external.kind.clone(),
                abi: external.abi,
                params: external
                    .params
                    .iter()
                    .map(|param| ParamSig {
                        name: param.name.clone(),
                        ty: param.ty.clone(),
                    })
                    .collect(),
            },
        )?;
    }
    for routine in &source.routines {
        insert_routine_sig(
            &mut out,
            &routine.name,
            RoutineSig {
                kind: routine.kind.clone(),
                abi: routine.abi,
                params: routine
                    .params
                    .iter()
                    .map(|param| ParamSig {
                        name: param.name.clone(),
                        ty: param.ty.clone(),
                    })
                    .collect(),
            },
        )?;
    }
    Ok(out)
}

fn insert_routine_sig(
    out: &mut BTreeMap<String, RoutineSig>,
    name: &str,
    sig: RoutineSig,
) -> Result<(), FrontendError> {
    if out.insert(name.to_owned(), sig).is_some() {
        Err(FrontendError::Type(format!("duplicate routine {name}")))
    } else {
        Ok(())
    }
}

impl Lowerer {
    fn lower_routine(
        &mut self,
        source: &RoutineDecl,
        globals: &[GlobalDecl],
        consts: &[ConstDecl],
        entry: Option<&EntryDecl>,
    ) -> Result<Routine, FrontendError> {
        let kind = match &source.kind {
            RoutineDeclKind::Procedure => RoutineKind::Procedure,
            RoutineDeclKind::Function { result } => RoutineKind::Function {
                result: result.clone(),
            },
        };
        let mut routine = Routine {
            name: source.name.clone(),
            kind,
            export: source.export || entry.is_some_and(|entry| entry.routine == source.name),
            abi: if entry.is_some_and(|entry| entry.routine == source.name)
                && matches!(source.abi, Abi::Internal)
            {
                Abi::Exported
            } else {
                source.abi
            },
            interrupt_preserve: source.interrupt_preserve,
            interrupt_raw: source.interrupt_raw,
            interrupt_cop: source.interrupt_cop,
            params: source
                .params
                .iter()
                .map(|param| Param {
                    name: param.name.clone(),
                    ty: param.ty.clone(),
                    storage: param.storage,
                })
                .collect(),
            locals: source
                .locals
                .iter()
                .map(|local| Local {
                    name: local.name.clone(),
                    ty: local.ty.clone(),
                    storage: local.storage,
                })
                .collect(),
            stack_frame_bytes: 0,
            body: Vec::new(),
        };

        let mut lowerer = RoutineLowerer::new(self, source, globals, consts)?;
        if let Some(param) = source.params.iter().find(|param| {
            !matches!(
                param.ty,
                Type::U8
                    | Type::I8
                    | Type::U16
                    | Type::I16
                    | Type::U24
                    | Type::U32
                    | Type::I32
                    | Type::Pointer { .. }
                    | Type::Routine(_)
            )
        }) {
            return Err(FrontendError::Type(format!(
                "routine {} parameter {} has unsupported type {:?}",
                source.name, param.name, param.ty
            )));
        }
        for stmt in &source.body {
            lowerer.lower_stmt(stmt)?;
        }
        routine.body = lowerer.body;
        Ok(routine)
    }

    fn validate_vectors_and_entry(&self, module: &Module) -> Result<(), FrontendError> {
        for vector in &module.vectors {
            let Some(sig) = self.routines.get(&vector.routine) else {
                return Err(FrontendError::Type(format!(
                    "vector {} references unknown routine {}",
                    vector.name, vector.routine
                )));
            };
            if let Some(family) = trap_vector_family(&vector.name) {
                // Trap vectors point at a family dispatcher with the trap ABI:
                // reentrant function(call: u16, arg0: u16, arg1: u16): u16.
                family.map_err(FrontendError::Type)?;
                if !matches!(sig.abi, Abi::Reentrant) {
                    return Err(FrontendError::Type(format!(
                        "trap vector {} references non-reentrant routine {}",
                        vector.name, vector.routine
                    )));
                }
            } else if !matches!(sig.abi, Abi::Interrupt) {
                return Err(FrontendError::Type(format!(
                    "vector {} references non-interrupt routine {}",
                    vector.name, vector.routine
                )));
            }
        }
        if let Some(entry) = &module.entry {
            let Some(sig) = self.routines.get(&entry.routine) else {
                return Err(FrontendError::Type(format!(
                    "entry references unknown routine {}",
                    entry.routine
                )));
            };
            if matches!(sig.abi, Abi::Interrupt) {
                return Err(FrontendError::Type(format!(
                    "entry references interrupt routine {}",
                    entry.routine
                )));
            }
        }
        Ok(())
    }

    fn label(&mut self, prefix: &str) -> Label {
        let id = self.labels;
        self.labels += 1;
        Label::new(format!("__{prefix}_{id}"))
    }

    fn temp(&mut self) -> Value {
        let id = self.temps;
        self.temps += 1;
        Value::Temp(format!("__tmp{id}"))
    }

    /// Appends the data for `init`, a value of type `ty`, to `out`.
    fn encode_init(
        &mut self,
        ty: &Type,
        init: &Init,
        consts: &BTreeMap<String, i64>,
        globals: &[&str],
        out: &mut Vec<DataItem>,
    ) -> Result<(), FrontendError> {
        match (ty, init) {
            (Type::Array { len, element }, Init::List(items)) => {
                if items.len() != usize::from(*len) {
                    return Err(FrontendError::Type(format!(
                        "{} elements for a {len}-element array",
                        items.len()
                    )));
                }
                for item in items {
                    self.encode_init(element, item, consts, globals, out)?;
                }
                Ok(())
            }
            (Type::Array { len, element }, Init::Expr(Expr::String(text)))
                if element.size_bytes() == 1 =>
            {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                if bytes.len() > usize::from(*len) {
                    return Err(FrontendError::Type(format!(
                        "the string needs {} bytes; the array has {len}",
                        bytes.len()
                    )));
                }
                bytes.resize(usize::from(*len), 0);
                push_data_bytes(out, &bytes);
                Ok(())
            }
            (Type::Record { fields }, Init::List(items)) => {
                if items.len() != fields.len() {
                    return Err(FrontendError::Type(format!(
                        "{} values for a {}-field record",
                        items.len(),
                        fields.len()
                    )));
                }
                for (field, item) in fields.iter().zip(items) {
                    self.encode_init(&field.ty, item, consts, globals, out)?;
                }
                Ok(())
            }
            (_, Init::Expr(expr)) if integer_width(ty).is_some() => {
                let lookup = |name: &str| consts.get(name).copied();
                if is_const(expr, &lookup) {
                    let value = const_for_type(const_eval(expr, &lookup)?, ty)?;
                    let bytes: Vec<u8> = (0..ty.size_bytes()).map(|i| value.byte(i)).collect();
                    push_data_bytes(out, &bytes);
                    return Ok(());
                }
                let bytes = ty.size_bytes();
                if !matches!(ty, Type::Pointer { .. } | Type::Routine(_)) || bytes == 3 {
                    return Err(FrontendError::Type(format!(
                        "{expr:?} is not a constant {ty:?}"
                    )));
                }
                let symbol = match expr {
                    Expr::String(text) if matches!(ty, Type::Pointer { .. }) => {
                        self.intern_string(text)
                    }
                    Expr::Call { name, args } if name == "addr" => match args.as_slice() {
                        [Expr::Name(global)] if globals.contains(&global.as_str()) => {
                            global.clone()
                        }
                        _ => {
                            return Err(FrontendError::Type(
                                "addr expects one global variable name".to_owned(),
                            ))
                        }
                    },
                    Expr::Name(routine) if self.routines.contains_key(routine) => {
                        let sig = &self.routines[routine];
                        let pointer = Type::Routine(Box::new(routine_type_of(routine, sig)?));
                        ensure_assignable(ty, &pointer)?;
                        routine.clone()
                    }
                    _ => {
                        return Err(FrontendError::Type(format!(
                            "{expr:?} is not a constant {ty:?}"
                        )))
                    }
                };
                out.push(DataItem::Address { symbol, bytes });
                Ok(())
            }
            _ => Err(FrontendError::Type(format!(
                "initialiser does not match type {ty:?}"
            ))),
        }
    }

    fn intern_string(&mut self, value: &str) -> String {
        let id = self.strings.len();
        let name = format!("__str_{id}");
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        self.strings.push(StaticData::bytes(name.clone(), bytes));
        name
    }
}

impl<'a> RoutineLowerer<'a> {
    fn new(
        parent: &'a mut Lowerer,
        routine: &'a RoutineDecl,
        globals: &[GlobalDecl],
        consts: &[ConstDecl],
    ) -> Result<Self, FrontendError> {
        let mut bindings = BTreeMap::new();
        for const_decl in consts {
            insert_binding(
                &mut bindings,
                &const_decl.name,
                Binding {
                    ty: const_decl.ty.clone(),
                    value: Value::Const(const_for_type(const_decl.value, &const_decl.ty)?),
                    readonly: true,
                },
            )?;
        }
        for global in globals {
            insert_binding(
                &mut bindings,
                &global.name,
                Binding {
                    ty: global.ty.clone(),
                    value: Value::Global(global.name.clone()),
                    readonly: global.readonly,
                },
            )?;
        }
        for param in &routine.params {
            insert_binding(
                &mut bindings,
                &param.name,
                Binding {
                    ty: param.ty.clone(),
                    value: Value::Param(param.name.clone()),
                    readonly: false,
                },
            )?;
        }
        for local in &routine.locals {
            insert_binding(
                &mut bindings,
                &local.name,
                Binding {
                    ty: local.ty.clone(),
                    value: Value::Local(local.name.clone()),
                    readonly: false,
                },
            )?;
        }
        Ok(Self {
            parent,
            routine,
            bindings,
            body: Vec::new(),
        })
    }

    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<(), FrontendError> {
        match stmt {
            Stmt::InlineAsm(line) => {
                self.body.push(IrInst::InlineAsm(line.clone()));
                Ok(())
            }
            Stmt::Assign { target, value } => self.lower_assign(target, value),
            Stmt::Call { name, args } => {
                if let Some(routine) = self.routine_pointer(name) {
                    return self.lower_indirect_call(name, &routine, args, None);
                }
                let sig = self.check_call(name, args, true)?;
                let params: Vec<Type> = sig.params.iter().map(|param| param.ty.clone()).collect();
                self.lower_call_args(name, sig.abi, args, &sig.params)?;
                self.body.push(IrInst::CallProcedure {
                    name: name.clone(),
                    abi: sig.abi,
                });
                self.cleanup_call_args(sig.abi, &params);
                Ok(())
            }
            Stmt::If {
                cond,
                body,
                else_body,
            } => {
                let else_label = self.parent.label("if_else");
                let done = self.parent.label("if_done");
                let false_target = if else_body.is_empty() {
                    done.clone()
                } else {
                    else_label.clone()
                };
                self.lower_branch_if_false(cond, false_target)?;
                for stmt in body {
                    self.lower_stmt(stmt)?;
                }
                if !else_body.is_empty() {
                    self.body.push(IrInst::Branch {
                        target: done.clone(),
                    });
                    self.body.push(IrInst::Label(else_label));
                    for stmt in else_body {
                        self.lower_stmt(stmt)?;
                    }
                }
                self.body.push(IrInst::Label(done));
                Ok(())
            }
            Stmt::While { cond, body } => {
                let start = self.parent.label("while_start");
                let done = self.parent.label("while_done");
                self.body.push(IrInst::Label(start.clone()));
                self.lower_branch_if_false(cond, done.clone())?;
                for stmt in body {
                    self.lower_stmt(stmt)?;
                }
                self.body.push(IrInst::Branch { target: start });
                self.body.push(IrInst::Label(done));
                Ok(())
            }
            Stmt::Repeat { body, until } => {
                let start = self.parent.label("repeat_start");
                self.body.push(IrInst::Label(start.clone()));
                for stmt in body {
                    self.lower_stmt(stmt)?;
                }
                self.lower_branch_if_false(until, start)?;
                Ok(())
            }
            Stmt::For {
                var,
                start,
                end,
                down,
                body,
            } => self.lower_for(var, start, end, *down, body),
            Stmt::Return(value) => self.lower_return(value.as_ref()),
        }
    }

    fn lower_assign(&mut self, target: &Expr, value: &Expr) -> Result<(), FrontendError> {
        match target {
            Expr::Name(name) => {
                let binding = self.binding(name)?.clone();
                self.lower_expr_to(value, &binding.ty, binding.value)
            }
            Expr::Deref(_) | Expr::Index(..) | Expr::Field(..) => {
                if let Some(root) = place_root(target) {
                    if self.binding(root)?.readonly {
                        return Err(FrontendError::Type(format!(
                            "{root} is a constant table and cannot be assigned"
                        )));
                    }
                }
                let ty = self.infer_expr(target)?;
                // The value first: computing the address may use the accumulator.
                let value = self.expr_value(value, &ty)?;
                let (place, ty) = self.place(target)?;
                let addr = self.place_address(place)?;
                self.body.push(IrInst::Store {
                    width: width_for_type(&ty)?,
                    addr,
                    src: value,
                });
                Ok(())
            }
            _ => Err(FrontendError::Type(
                "assignment target is not assignable".to_owned(),
            )),
        }
    }

    /// `for var = start to end`: skips the body when start is already past
    /// end, and stops after the pass where var equals end, so a loop up to a
    /// type's largest value ends.
    fn lower_for(
        &mut self,
        var: &str,
        start: &Expr,
        end: &Expr,
        down: bool,
        body: &[Stmt],
    ) -> Result<(), FrontendError> {
        let counter = Expr::Name(var.to_owned());
        if integer_width(&self.binding(var)?.ty).is_none() {
            return Err(FrontendError::Type(format!(
                "for loop variable {var} needs an integer type"
            )));
        }
        let top = self.parent.label("for_top");
        let done = self.parent.label("for_done");
        self.lower_assign(&counter, start)?;
        let compare = |op| {
            Cond::Compare(CondCompare {
                lhs: counter.clone(),
                op,
                rhs: end.clone(),
            })
        };
        self.lower_branch_if_true(
            &compare(if down { CondOp::Lt } else { CondOp::Gt }),
            done.clone(),
        )?;
        self.body.push(IrInst::Label(top.clone()));
        for stmt in body {
            self.lower_stmt(stmt)?;
        }
        self.lower_branch_if_true(&compare(CondOp::Eq), done.clone())?;
        let step = if down {
            Expr::Sub(Box::new(counter.clone()), Box::new(Expr::Number(1)))
        } else {
            Expr::Add(Box::new(counter.clone()), Box::new(Expr::Number(1)))
        };
        self.lower_assign(&counter, &step)?;
        self.body.push(IrInst::Branch { target: top });
        self.body.push(IrInst::Label(done));
        Ok(())
    }

    fn lower_return(&mut self, value: Option<&Expr>) -> Result<(), FrontendError> {
        match (&self.routine.kind, value) {
            (RoutineDeclKind::Procedure, None) => {
                self.body.push(IrInst::Return);
                Ok(())
            }
            (RoutineDeclKind::Procedure, Some(_)) => Err(FrontendError::Type(
                "procedure cannot return a value".to_owned(),
            )),
            (RoutineDeclKind::Function { result }, Some(expr)) => {
                let value = self.expr_value(expr, result).or_else(|error| {
                    if !matches!(expr, Expr::ResultOk(_) | Expr::ResultErr(_)) {
                        return Err(error);
                    }
                    self.lower_expr_to(expr, result, Value::Accumulator)?;
                    Ok(Value::Accumulator)
                })?;
                self.body.push(IrInst::ReturnValue(value));
                Ok(())
            }
            (RoutineDeclKind::Function { .. }, None) => Err(FrontendError::Type(
                "function return needs a value".to_owned(),
            )),
        }
    }

    fn lower_branch_if_false(
        &mut self,
        cond: &Cond,
        false_target: Label,
    ) -> Result<(), FrontendError> {
        match cond {
            Cond::Compare(compare) => self.lower_compare_branch(compare, false_target, false),
            Cond::And(lhs, rhs) => {
                self.lower_branch_if_false(lhs, false_target.clone())?;
                self.lower_branch_if_false(rhs, false_target)
            }
            Cond::Or(lhs, rhs) => {
                let true_target = self.parent.label("cond_true");
                self.lower_branch_if_true(lhs, true_target.clone())?;
                self.lower_branch_if_false(rhs, false_target)?;
                self.body.push(IrInst::Label(true_target));
                Ok(())
            }
            Cond::Not(cond) => self.lower_branch_if_true(cond, false_target),
        }
    }

    fn lower_branch_if_true(
        &mut self,
        cond: &Cond,
        true_target: Label,
    ) -> Result<(), FrontendError> {
        match cond {
            Cond::Compare(compare) => self.lower_compare_branch(compare, true_target, true),
            Cond::And(lhs, rhs) => {
                let false_target = self.parent.label("cond_false");
                self.lower_branch_if_false(lhs, false_target.clone())?;
                self.lower_branch_if_true(rhs, true_target)?;
                self.body.push(IrInst::Label(false_target));
                Ok(())
            }
            Cond::Or(lhs, rhs) => {
                self.lower_branch_if_true(lhs, true_target.clone())?;
                self.lower_branch_if_true(rhs, true_target)
            }
            Cond::Not(cond) => self.lower_branch_if_false(cond, true_target),
        }
    }

    fn lower_compare_branch(
        &mut self,
        cond: &CondCompare,
        target: Label,
        branch_when_true: bool,
    ) -> Result<(), FrontendError> {
        let mut lhs_ty = self.infer_expr(&cond.lhs)?;
        let rhs_ty = self.infer_expr(&cond.rhs)?;
        // `0 < x` compares at the width (and signedness) of `x`.
        if self.is_const_expr(&cond.lhs)? && !self.is_const_expr(&cond.rhs)? {
            compatible_expr_width(&rhs_ty, &lhs_ty, true)?;
            lhs_ty = rhs_ty.clone();
        }
        let width = compatible_expr_width(&lhs_ty, &rhs_ty, self.is_const_expr(&cond.rhs)?)?;
        // Equality does not depend on signedness.
        let signed = (lhs_ty.is_signed() || rhs_ty.is_signed())
            && !matches!(cond.op, CondOp::Eq | CondOp::Ne);
        // The left side may stay in the accumulator only if evaluating the
        // right side cannot disturb it.
        let rhs_is_simple = matches!(cond.rhs, Expr::Number(_) | Expr::Name(_));
        let lhs = if matches!(cond.lhs, Expr::Number(_) | Expr::String(_) | Expr::Name(_))
            || !rhs_is_simple
        {
            self.expr_value(&cond.lhs, &lhs_ty)?
        } else {
            self.lower_expr_to(&cond.lhs, &lhs_ty, Value::Accumulator)?;
            Value::Accumulator
        };
        let rhs = self.expr_value(&cond.rhs, &lhs_ty)?;
        self.body.push(IrInst::Compare {
            width,
            signed,
            lhs,
            rhs,
        });
        self.lower_compare_result_branch(cond.op, target, branch_when_true);
        Ok(())
    }

    fn lower_compare_result_branch(&mut self, op: CondOp, target: Label, branch_when_true: bool) {
        match (op, branch_when_true) {
            (CondOp::Eq, true) | (CondOp::Ne, false) => {
                self.body.push(IrInst::BranchIfEqual { target });
            }
            (CondOp::Ne, true) | (CondOp::Eq, false) => {
                self.body.push(IrInst::BranchIfNotEqual { target });
            }
            (CondOp::Lt, true) | (CondOp::Ge, false) => {
                self.body.push(IrInst::BranchIfCarryClear { target });
            }
            (CondOp::Ge, true) | (CondOp::Lt, false) => {
                self.body.push(IrInst::BranchIfCarrySet { target });
            }
            (CondOp::Gt, true) | (CondOp::Le, false) => {
                let skip = self.parent.label("compare_skip");
                self.body.push(IrInst::BranchIfCarryClear {
                    target: skip.clone(),
                });
                self.body.push(IrInst::BranchIfEqual {
                    target: skip.clone(),
                });
                self.body.push(IrInst::Branch { target });
                self.body.push(IrInst::Label(skip));
            }
            (CondOp::Le, true) | (CondOp::Gt, false) => {
                self.body.push(IrInst::BranchIfCarryClear {
                    target: target.clone(),
                });
                self.body.push(IrInst::BranchIfEqual { target });
            }
        }
    }

    fn lower_expr_to(
        &mut self,
        expr: &Expr,
        expected: &Type,
        dst: Value,
    ) -> Result<(), FrontendError> {
        if !matches!(expr, Expr::Number(_) | Expr::Name(_)) && self.is_const_expr(expr)? {
            let value = self.const_expr_value(expr)?;
            self.body.push(IrInst::LoadConst {
                dst,
                value: const_for_type(value, expected)?,
            });
            return Ok(());
        }
        match expr {
            Expr::Number(value) => {
                self.body.push(IrInst::LoadConst {
                    dst,
                    value: const_for_type(i64::from(*value), expected)?,
                });
                Ok(())
            }
            Expr::String(value) => {
                ensure_assignable(expected, &string_literal_type())?;
                let symbol = self.parent.intern_string(value);
                self.body.push(IrInst::LoadAddress {
                    dst,
                    symbol,
                    width: width_for_type(expected)?,
                });
                Ok(())
            }
            Expr::Name(name) => {
                if let Some(routine) = self.routine_address(name)? {
                    ensure_assignable(expected, &Type::Routine(Box::new(routine)))?;
                    self.body.push(IrInst::LoadAddress {
                        dst,
                        symbol: name.clone(),
                        width: Width::Word,
                    });
                    return Ok(());
                }
                let binding = self.binding(name)?.clone();
                if let Value::Const(value) = binding.value {
                    self.body.push(IrInst::LoadConst {
                        dst,
                        value: const_for_type(const_value(value, &binding.ty), expected)?,
                    });
                    return Ok(());
                }
                ensure_assignable(expected, &binding.ty)?;
                self.body.push(IrInst::Move {
                    width: width_for_type(expected)?,
                    dst,
                    src: binding.value,
                });
                Ok(())
            }
            Expr::Call { name, args } => {
                if name == "addr" {
                    let symbol = self.addr_builtin_symbol(args)?;
                    ensure_assignable(expected, &addr_builtin_type())?;
                    self.body.push(IrInst::LoadAddress {
                        dst,
                        symbol,
                        width: Width::Word,
                    });
                    return Ok(());
                }
                if let Some(target) = integer_cast_builtin(name) {
                    self.lower_integer_cast(name, args, &target, expected, dst)?;
                    return Ok(());
                }
                if name == "cop_signature" {
                    self.check_cop_signature_call(args)?;
                    ensure_assignable(expected, &Type::U8)?;
                    self.body.push(IrInst::CopSignature { dst });
                    return Ok(());
                }
                if let Some(register) = cop_register_builtin(name) {
                    self.check_cop_register_call(name, args)?;
                    ensure_assignable(expected, &Type::U16)?;
                    self.body.push(IrInst::CopRegister { register, dst });
                    return Ok(());
                }
                if is_trap_builtin(name) {
                    self.lower_cop_call(args, expected, dst)?;
                    return Ok(());
                }
                if let Some(routine) = self.routine_pointer(name) {
                    return self.lower_indirect_call(name, &routine, args, Some((expected, dst)));
                }
                let sig = self.check_call(name, args, false)?;
                let result = sig.result_type(name)?;
                ensure_assignable(expected, &result)?;
                let params: Vec<Type> = sig.params.iter().map(|param| param.ty.clone()).collect();
                self.lower_call_args(name, sig.abi, args, &sig.params)?;
                let capture_before_cleanup = matches!(sig.abi, Abi::Reentrant)
                    && !sig.params.is_empty()
                    && (matches!(dst, Value::Accumulator | Value::Stack { .. })
                        || (matches!(self.routine.abi, Abi::Reentrant)
                            && matches!(dst, Value::Temp(_) | Value::Local(_) | Value::Param(_))));
                let call_dst = if capture_before_cleanup {
                    Value::IndexX
                } else {
                    dst.clone()
                };
                self.body.push(IrInst::CallFunction {
                    name: name.clone(),
                    abi: sig.abi,
                    width: width_for_type(&result)?,
                    dst: call_dst.clone(),
                });
                self.cleanup_call_args(sig.abi, &params);
                if capture_before_cleanup {
                    self.body.push(IrInst::Move {
                        width: width_for_type(&result)?,
                        dst,
                        src: call_dst,
                    });
                }
                Ok(())
            }
            Expr::ResultOk(value) => {
                let Type::Result { ok, .. } = expected else {
                    return Err(FrontendError::Type(
                        "ok constructor needs result type".to_owned(),
                    ));
                };
                let encoded = self.result_constructor_const(value, ok, 0)?;
                self.body.push(IrInst::LoadConst {
                    dst,
                    value: Const::U16(encoded),
                });
                Ok(())
            }
            Expr::ResultErr(value) => {
                let Type::Result { err, .. } = expected else {
                    return Err(FrontendError::Type(
                        "err constructor needs result type".to_owned(),
                    ));
                };
                let encoded = self.result_constructor_const(value, err, 1)?;
                self.body.push(IrInst::LoadConst {
                    dst,
                    value: Const::U16(encoded),
                });
                Ok(())
            }
            Expr::Add(lhs, rhs) => {
                let (lhs_ty, width) = self.operand_type(lhs, rhs)?;
                ensure_assignable(expected, &lhs_ty)?;
                let lhs = self.expr_value(lhs, &lhs_ty)?;
                let rhs = self.expr_value(rhs, &lhs_ty)?;
                self.body.push(IrInst::Add {
                    width,
                    dst,
                    lhs,
                    rhs,
                });
                Ok(())
            }
            Expr::Sub(lhs, rhs) => {
                let (lhs_ty, width) = self.operand_type(lhs, rhs)?;
                ensure_assignable(expected, &lhs_ty)?;
                let lhs = self.expr_value(lhs, &lhs_ty)?;
                let rhs = self.expr_value(rhs, &lhs_ty)?;
                self.body.push(IrInst::Sub {
                    width,
                    dst,
                    lhs,
                    rhs,
                });
                Ok(())
            }
            Expr::Mul(lhs, rhs)
            | Expr::Div(lhs, rhs)
            | Expr::Rem(lhs, rhs)
            | Expr::BitXor(lhs, rhs) => self.lower_binary(expr, lhs, rhs, expected, dst),
            Expr::Neg(value) => {
                let ty = self.infer_expr(value)?;
                let width = width_for_type(&ty)?;
                ensure_assignable(expected, &ty)?;
                let value = self.expr_value(value, &ty)?;
                self.body.push(IrInst::Sub {
                    width,
                    dst,
                    lhs: Value::Const(wrap_const(0, width)),
                    rhs: value,
                });
                Ok(())
            }
            Expr::BitAnd(lhs, rhs) => {
                let (lhs_ty, width) = self.operand_type(lhs, rhs)?;
                ensure_assignable(expected, &lhs_ty)?;
                let lhs = self.expr_value(lhs, &lhs_ty)?;
                let rhs = self.expr_value(rhs, &lhs_ty)?;
                self.body.push(IrInst::BitAnd {
                    width,
                    dst,
                    lhs,
                    rhs,
                });
                Ok(())
            }
            Expr::BitOr(lhs, rhs) => {
                let (lhs_ty, width) = self.operand_type(lhs, rhs)?;
                ensure_assignable(expected, &lhs_ty)?;
                let lhs = self.expr_value(lhs, &lhs_ty)?;
                let rhs = self.expr_value(rhs, &lhs_ty)?;
                self.body.push(IrInst::BitOr {
                    width,
                    dst,
                    lhs,
                    rhs,
                });
                Ok(())
            }
            Expr::ShiftLeft(lhs, rhs) | Expr::ShiftRight(lhs, rhs) => {
                let lhs_ty = self.infer_expr(lhs)?;
                let count = self.shift_count(&lhs_ty, rhs)?;
                let width = width_for_type(&lhs_ty)?;
                ensure_assignable(expected, &lhs_ty)?;
                let signed = lhs_ty.is_signed();
                let lhs = self.expr_value(lhs, &lhs_ty)?;
                let rhs = Value::Const(Const::U8(count));
                self.body.push(if matches!(expr, Expr::ShiftLeft(..)) {
                    IrInst::ShiftLeft {
                        width,
                        dst,
                        lhs,
                        rhs,
                    }
                } else if signed {
                    IrInst::Binary {
                        op: BinaryOp::ShiftRightSigned,
                        width,
                        dst,
                        lhs,
                        rhs,
                    }
                } else {
                    IrInst::ShiftRight {
                        width,
                        dst,
                        lhs,
                        rhs,
                    }
                });
                Ok(())
            }
            Expr::Deref(_) | Expr::Index(..) | Expr::Field(..) => {
                let (place, ty) = self.place(expr)?;
                ensure_assignable(expected, &ty)?;
                let addr = self.place_address(place)?;
                self.body.push(IrInst::Load {
                    width: width_for_type(&ty)?,
                    dst,
                    addr,
                });
                Ok(())
            }
        }
    }

    fn expr_value(&mut self, expr: &Expr, expected: &Type) -> Result<Value, FrontendError> {
        if integer_width(expected).is_some() && self.is_const_expr(expr)? {
            return Ok(Value::Const(const_for_type(
                self.const_expr_value(expr)?,
                expected,
            )?));
        }
        match expr {
            Expr::Number(value) => Ok(Value::Const(const_for_type(i64::from(*value), expected)?)),
            Expr::String(value) => {
                ensure_assignable(expected, &string_literal_type())?;
                let symbol = self.parent.intern_string(value);
                let temp = self.parent.temp();
                self.body.push(IrInst::LoadAddress {
                    dst: temp.clone(),
                    symbol,
                    width: width_for_type(expected)?,
                });
                Ok(temp)
            }
            Expr::Name(name) if self.bindings.contains_key(name) => {
                let binding = self.binding(name)?;
                if let Value::Const(value) = binding.value {
                    return Ok(Value::Const(const_for_type(
                        const_value(value, &binding.ty),
                        expected,
                    )?));
                }
                ensure_assignable(expected, &binding.ty)?;
                Ok(binding.value.clone())
            }
            Expr::ResultOk(_) | Expr::ResultErr(_) => Err(FrontendError::Type(
                "result constructor is not an addressable value".to_owned(),
            )),
            _ => {
                let temp = self.parent.temp();
                self.lower_expr_to(expr, expected, temp.clone())?;
                Ok(temp)
            }
        }
    }

    /// Where an element, field or pointer target lives (see [`Place`]).
    fn place(&mut self, expr: &Expr) -> Result<(Place, Type), FrontendError> {
        match expr {
            Expr::Name(name) => {
                let binding = self.binding(name)?.clone();
                match binding.value {
                    Value::Global(symbol) => Ok((
                        Place::Global {
                            symbol,
                            offset: 0,
                            index: None,
                            whole: binding.ty.size_bytes(),
                        },
                        binding.ty,
                    )),
                    Value::Const(_) => Err(FrontendError::Type(format!(
                        "constant {name} has no address"
                    ))),
                    base => Ok((Place::Value { base, offset: 0 }, binding.ty)),
                }
            }
            Expr::Field(base, name) => {
                let (mut place, ty) = self.place(base)?;
                let field = record_field(&ty, name)?.clone();
                place.add_offset(field.offset)?;
                Ok((place, field.ty))
            }
            Expr::Index(base, index) => {
                let (mut place, ty) = self.place(base)?;
                let Type::Array { len, element } = ty else {
                    return Err(FrontendError::Type(
                        "indexed access needs an array".to_owned(),
                    ));
                };
                let size = element.size_bytes();
                if self.is_const_expr(index)? {
                    let at = self.const_expr_value(index)?;
                    if !(0..i64::from(len)).contains(&at) {
                        return Err(FrontendError::Type(format!(
                            "index {at} is outside the {len}-element array"
                        )));
                    }
                    place.add_offset(at as u16 * size)?;
                    return Ok((place, *element));
                }
                // Evaluating the index may need the accumulator.
                self.spill_place_index(&mut place);
                let offset = self.index_offset(index, size)?;
                self.add_place_index(&mut place, offset)?;
                Ok((place, *element))
            }
            Expr::Deref(pointer) => {
                let ty = self.infer_expr(pointer)?;
                let Type::Pointer { to, kind } = ty.clone() else {
                    return Err(FrontendError::Type(
                        "dereference needs a pointer".to_owned(),
                    ));
                };
                let value = self.expr_value(pointer, &ty)?;
                let pointer = if matches!(
                    value,
                    Value::Direct(_)
                        | Value::Stack { .. }
                        | Value::Temp(_)
                        | Value::Local(_)
                        | Value::Param(_)
                ) {
                    value
                } else {
                    let temp = self.parent.temp();
                    self.body.push(IrInst::Move {
                        width: width_for_type(&ty)?,
                        dst: temp.clone(),
                        src: value,
                    });
                    temp
                };
                Ok((
                    Place::Pointer {
                        pointer,
                        far: matches!(kind, PointerKind::Far),
                        offset: 0,
                        index: None,
                    },
                    *to,
                ))
            }
            _ => Err(FrontendError::Type(
                "this expression has no address".to_owned(),
            )),
        }
    }

    /// The byte offset of element `index` (a 1- or 2-byte integer) in
    /// elements of `size` bytes, as a 16-bit value.
    fn index_offset(&mut self, index: &Expr, size: u16) -> Result<Value, FrontendError> {
        let ty = self.infer_expr(index)?;
        let value = match integer_width(&ty) {
            Some(2) if matches!(index, Expr::Name(_)) => self.expr_value(index, &ty)?,
            Some(2) => {
                self.lower_expr_to(index, &ty, Value::Accumulator)?;
                Value::Accumulator
            }
            Some(1) => {
                let byte = self.expr_value(index, &ty)?;
                self.body.push(IrInst::LoadConst {
                    dst: Value::Accumulator,
                    value: Const::U16(0),
                });
                self.body.push(IrInst::Move {
                    width: Width::Byte,
                    dst: Value::Accumulator,
                    src: byte,
                });
                Value::Accumulator
            }
            _ => {
                return Err(FrontendError::Type(
                    "array indexes need an 8- or 16-bit integer".to_owned(),
                ))
            }
        };
        self.scale_index(value, size)
    }

    /// Moves a place's index out of the accumulator.
    fn spill_place_index(&mut self, place: &mut Place) {
        if let Place::Global { index, .. } | Place::Pointer { index, .. } = place {
            if *index == Some(Value::Accumulator) {
                let temp = self.parent.temp();
                self.body.push(IrInst::Move {
                    width: Width::Word,
                    dst: temp.clone(),
                    src: Value::Accumulator,
                });
                *index = Some(temp);
            }
        }
    }

    fn add_place_index(&mut self, place: &mut Place, offset: Value) -> Result<(), FrontendError> {
        let index = match place {
            Place::Global { index, .. } | Place::Pointer { index, .. } => index,
            Place::Value { .. } => {
                return Err(FrontendError::Type(
                    "indexing a local array needs a constant index (make the array global)"
                        .to_owned(),
                ))
            }
        };
        *index = Some(match index.take() {
            None => offset,
            // The accumulator, if used, must be the left side.
            Some(previous) => {
                let (lhs, rhs) = if offset == Value::Accumulator {
                    (offset, previous)
                } else {
                    (previous, offset)
                };
                self.body.push(IrInst::Add {
                    width: Width::Word,
                    dst: Value::Accumulator,
                    lhs,
                    rhs,
                });
                Value::Accumulator
            }
        });
        Ok(())
    }

    /// The IR address of a place.
    fn place_address(&mut self, place: Place) -> Result<Address, FrontendError> {
        Ok(match place {
            Place::Global {
                symbol,
                offset,
                index: None,
                ..
            } => Address::ValueOffset {
                base: Value::Global(symbol),
                offset,
            },
            Place::Global {
                symbol,
                offset,
                index: Some(index),
                whole,
            } => Address::Indexed {
                base: if offset == 0 {
                    symbol
                } else {
                    format!("{symbol}+{offset}")
                },
                index,
                small: whole <= 0x100,
            },
            Place::Value { base, offset } => Address::ValueOffset { base, offset },
            Place::Pointer {
                pointer,
                far,
                offset: 0,
                index: None,
            } => {
                if far {
                    Address::FarPtr(pointer)
                } else {
                    Address::NearPtr(pointer)
                }
            }
            Place::Pointer {
                pointer,
                far,
                offset,
                index,
            } => {
                // Form pointer + index + offset in a temporary.
                let (width, offset_value) = if far {
                    (Width::Quad, Const::U32(u32::from(offset)))
                } else {
                    (Width::Word, Const::U16(offset))
                };
                let temp = self.parent.temp();
                match index {
                    Some(index) if far => {
                        self.body.push(IrInst::LoadConst {
                            dst: temp.clone(),
                            value: Const::U32(0),
                        });
                        self.body.push(IrInst::Move {
                            width: Width::Word,
                            dst: temp.clone(),
                            src: index,
                        });
                        self.body.push(IrInst::Add {
                            width,
                            dst: temp.clone(),
                            lhs: temp.clone(),
                            rhs: pointer,
                        });
                    }
                    Some(index) => self.body.push(IrInst::Add {
                        width,
                        dst: temp.clone(),
                        lhs: index,
                        rhs: pointer,
                    }),
                    None => self.body.push(IrInst::Move {
                        width,
                        dst: temp.clone(),
                        src: pointer,
                    }),
                }
                if offset != 0 {
                    self.body.push(IrInst::Add {
                        width,
                        dst: temp.clone(),
                        lhs: temp.clone(),
                        rhs: Value::Const(offset_value),
                    });
                }
                if far {
                    Address::FarPtr(temp)
                } else {
                    Address::NearPtr(temp)
                }
            }
        })
    }

    /// Turns an element index into a byte offset: `index * element_size`,
    /// as shifts and adds.
    fn scale_index(&mut self, index: Value, element_size: u16) -> Result<Value, FrontendError> {
        if element_size == 1 {
            return Ok(index);
        }
        if let Value::Const(value) = index {
            let offset = u32::from(value.bits() as u16) * u32::from(element_size);
            return Ok(Value::Const(Const::U16(offset as u16)));
        }
        let source = if matches!(index, Value::Accumulator) {
            let temp = self.parent.temp();
            self.body.push(IrInst::Move {
                width: Width::Word,
                dst: temp.clone(),
                src: Value::Accumulator,
            });
            temp
        } else {
            index
        };
        let mut first = true;
        for bit in (0..16u8).rev() {
            if element_size & (1 << bit) == 0 {
                continue;
            }
            let term = if bit == 0 {
                source.clone()
            } else {
                let dst = if first {
                    Value::Accumulator
                } else {
                    self.parent.temp()
                };
                self.body.push(IrInst::ShiftLeft {
                    width: Width::Word,
                    dst: dst.clone(),
                    lhs: source.clone(),
                    rhs: Value::Const(Const::U8(bit)),
                });
                dst
            };
            if first {
                if term != Value::Accumulator {
                    self.body.push(IrInst::Move {
                        width: Width::Word,
                        dst: Value::Accumulator,
                        src: term,
                    });
                }
                first = false;
            } else {
                self.body.push(IrInst::Add {
                    width: Width::Word,
                    dst: Value::Accumulator,
                    lhs: Value::Accumulator,
                    rhs: term,
                });
            }
        }
        Ok(Value::Accumulator)
    }

    fn infer_expr(&self, expr: &Expr) -> Result<Type, FrontendError> {
        match expr {
            Expr::Number(value) => Ok(const_type(i64::from(*value))),
            Expr::String(_) => Ok(string_literal_type()),
            Expr::Name(name) => match self.routine_address(name)? {
                Some(routine) => Ok(Type::Routine(Box::new(routine))),
                None => Ok(self.binding(name)?.ty.clone()),
            },
            Expr::Neg(value) if self.is_const_expr(value)? => {
                Ok(const_type(-self.const_expr_value(value)?))
            }
            Expr::Neg(value) => self.infer_expr(value),
            Expr::Call { name, args } if self.routine_pointer(name).is_some() => {
                let routine = self.routine_pointer(name).expect("checked above");
                self.check_indirect_call(name, &routine, args, false)?;
                Ok(routine.result.clone().expect("checked: a function"))
            }
            Expr::Call { name, args } if name == "addr" => {
                self.addr_builtin_symbol(args)?;
                Ok(addr_builtin_type())
            }
            Expr::Call { name, args } if integer_cast_builtin(name).is_some() => {
                self.check_integer_cast(name, args)?;
                Ok(integer_cast_builtin(name).expect("checked above"))
            }
            Expr::Call { name, args } if name == "cop_signature" => {
                self.check_cop_signature_call(args)?;
                Ok(Type::U8)
            }
            Expr::Call { name, args } if cop_register_builtin(name).is_some() => {
                self.check_cop_register_call(name, args)?;
                Ok(Type::U16)
            }
            Expr::Call { name, args } if is_trap_builtin(name) => {
                self.check_cop_call(args)?;
                Ok(Type::U16)
            }
            Expr::Call { name, args } => self.check_call(name, args, false)?.result_type(name),
            Expr::ResultOk(value) => Ok(Type::Result {
                ok: Box::new(self.infer_expr(value)?),
                err: Box::new(Type::U8),
            }),
            Expr::ResultErr(value) => Ok(Type::Result {
                ok: Box::new(Type::U8),
                err: Box::new(self.infer_expr(value)?),
            }),
            Expr::Add(lhs, rhs) | Expr::Sub(lhs, rhs) => Ok(self.operand_type(lhs, rhs)?.0),
            Expr::BitAnd(lhs, rhs)
            | Expr::BitOr(lhs, rhs)
            | Expr::BitXor(lhs, rhs)
            | Expr::Mul(lhs, rhs)
            | Expr::Div(lhs, rhs)
            | Expr::Rem(lhs, rhs) => Ok(self.operand_type(lhs, rhs)?.0),
            Expr::ShiftLeft(lhs, rhs) | Expr::ShiftRight(lhs, rhs) => {
                let lhs_ty = self.infer_expr(lhs)?;
                self.shift_count(&lhs_ty, rhs)?;
                Ok(lhs_ty)
            }
            Expr::Deref(ptr) => {
                let ty = self.infer_expr(ptr)?;
                match ty {
                    Type::Pointer { to, .. } => Ok(*to),
                    _ => Err(FrontendError::Type(
                        "dereference needs a pointer".to_owned(),
                    )),
                }
            }
            Expr::Index(base, index) => {
                let base_ty = self.infer_expr(base)?;
                if !self.is_const_expr(index)?
                    && !matches!(integer_width(&self.infer_expr(index)?), Some(1 | 2))
                {
                    return Err(FrontendError::Type(
                        "array indexes need an 8- or 16-bit integer".to_owned(),
                    ));
                }
                match base_ty {
                    Type::Array { element, .. } => Ok(*element),
                    _ => Err(FrontendError::Type(
                        "indexed access needs an array".to_owned(),
                    )),
                }
            }
            Expr::Field(base, field) => {
                let base_ty = self.infer_expr(base)?;
                Ok(record_field(&base_ty, field)?.ty.clone())
            }
        }
    }

    fn check_call(
        &self,
        name: &str,
        args: &[Expr],
        statement: bool,
    ) -> Result<RoutineSig, FrontendError> {
        let sig = self
            .parent
            .routines
            .get(name)
            .cloned()
            .ok_or_else(|| FrontendError::Type(format!("unknown routine {name}")))?;
        if args.len() != sig.params.len() {
            return Err(FrontendError::Type(format!(
                "routine {name} expected {} arguments, got {}",
                sig.params.len(),
                args.len()
            )));
        }
        if matches!(self.routine.abi, Abi::Interrupt)
            && !sig.params.is_empty()
            && !matches!(sig.abi, Abi::Reentrant)
        {
            return Err(FrontendError::Type(format!(
                "interrupt routine {} cannot call parameterized routine {name}; direct-page parameters are not interrupt-safe",
                self.routine.name
            )));
        }
        for (arg, param_ty) in args.iter().zip(&sig.params) {
            let arg_ty = self.infer_expr(arg)?;
            if self.is_const_expr(arg)? && integer_width(&param_ty.ty).is_some() {
                const_for_type(self.const_expr_value(arg)?, &param_ty.ty)?;
            } else {
                ensure_assignable(&param_ty.ty, &arg_ty)?;
            }
        }

        match (&sig.kind, statement) {
            (RoutineDeclKind::Procedure, true) => Ok(sig),
            (RoutineDeclKind::Procedure, false) => Err(FrontendError::Type(format!(
                "procedure {name} cannot be used as an expression"
            ))),
            (RoutineDeclKind::Function { .. }, true) => Err(FrontendError::Type(format!(
                "function {name} result cannot be discarded"
            ))),
            (RoutineDeclKind::Function { .. }, false) => Ok(sig),
        }
    }

    fn check_cop_signature_call(&self, args: &[Expr]) -> Result<(), FrontendError> {
        if !args.is_empty() {
            return Err(FrontendError::Type(format!(
                "cop_signature expected 0 arguments, got {}",
                args.len()
            )));
        }
        if !matches!(self.routine.abi, Abi::Interrupt) || !self.routine.interrupt_cop {
            return Err(FrontendError::Type(
                "cop_signature can only be used in interrupt(cop) routines".to_owned(),
            ));
        }
        Ok(())
    }

    fn check_cop_register_call(&self, name: &str, args: &[Expr]) -> Result<(), FrontendError> {
        if !args.is_empty() {
            return Err(FrontendError::Type(format!(
                "{name} expected 0 arguments, got {}",
                args.len()
            )));
        }
        if !matches!(self.routine.abi, Abi::Interrupt) || !self.routine.interrupt_cop {
            return Err(FrontendError::Type(format!(
                "{name} can only be used in interrupt(cop) routines"
            )));
        }
        Ok(())
    }

    fn check_integer_cast(&self, name: &str, args: &[Expr]) -> Result<(), FrontendError> {
        if args.len() != 1 {
            return Err(FrontendError::Type(format!(
                "{name} expected 1 argument, got {}",
                args.len()
            )));
        }
        if integer_width(&self.infer_expr(&args[0])?).is_none() {
            return Err(FrontendError::Type(format!(
                "{name} cast needs an integer argument"
            )));
        }
        Ok(())
    }

    fn lower_integer_cast(
        &mut self,
        name: &str,
        args: &[Expr],
        target: &Type,
        expected: &Type,
        dst: Value,
    ) -> Result<(), FrontendError> {
        self.check_integer_cast(name, args)?;
        ensure_assignable(expected, target)?;
        let width = width_for_type(target)?;
        // Casts of constants truncate (or sign-extend) at compile time.
        if self.is_const_expr(&args[0])? {
            self.body.push(IrInst::LoadConst {
                dst,
                value: wrap_const(self.const_expr_value(&args[0])?, width),
            });
            return Ok(());
        }
        let source_ty = self.infer_expr(&args[0])?;
        let source_width = width_for_type(&source_ty)?;
        let value = self.expr_value(&args[0], &source_ty)?;
        if width.size_bytes() <= source_width.size_bytes() {
            self.body.push(IrInst::Move {
                width,
                dst,
                src: value,
            });
        } else if source_ty.is_signed() {
            self.body.push(IrInst::SignExtend {
                width,
                src_width: source_width,
                dst,
                src: value,
            });
        } else {
            self.body.push(IrInst::LoadConst {
                dst: dst.clone(),
                value: wrap_const(0, width),
            });
            self.body.push(IrInst::Move {
                width: source_width,
                dst,
                src: value,
            });
        }
        Ok(())
    }

    fn check_cop_call(&self, args: &[Expr]) -> Result<(), FrontendError> {
        if args.len() != 4 {
            return Err(FrontendError::Type(format!(
                "cop_call expected 4 arguments, got {}",
                args.len()
            )));
        }
        let family = self.const_expr_value(&args[0])?;
        if !(0..=i64::from(u8::MAX)).contains(&family) {
            return Err(FrontendError::Type(format!(
                "COP family ${family:x} does not fit in one byte"
            )));
        }
        for arg in &args[1..] {
            let ty = self.infer_expr(arg)?;
            if self.is_const_expr(arg)? {
                const_for_type(self.const_expr_value(arg)?, &Type::U16)?;
            } else {
                ensure_assignable(&Type::U16, &ty)?;
            }
        }
        Ok(())
    }

    fn lower_cop_call(
        &mut self,
        args: &[Expr],
        expected: &Type,
        dst: Value,
    ) -> Result<(), FrontendError> {
        self.check_cop_call(args)?;
        ensure_assignable(expected, &Type::U16)?;
        let family = self.const_expr_value(&args[0])? as u8;
        let call = self.expr_value(&args[1], &Type::U16)?;
        let x = self.expr_value(&args[2], &Type::U16)?;
        let y = self.expr_value(&args[3], &Type::U16)?;
        self.body.push(IrInst::Trap {
            family,
            call,
            x,
            y,
            dst,
        });
        Ok(())
    }

    fn lower_call_args(
        &mut self,
        routine: &str,
        abi: Abi,
        args: &[Expr],
        params: &[ParamSig],
    ) -> Result<(), FrontendError> {
        if matches!(abi, Abi::Reentrant) {
            let types: Vec<Type> = params.iter().map(|param| param.ty.clone()).collect();
            return self.push_call_args(args, &types);
        }

        for (arg, param) in args.iter().zip(params) {
            self.lower_expr_to(
                arg,
                &param.ty,
                Value::Direct(crate::ir::param_symbol(routine, &param.name)),
            )?;
        }
        Ok(())
    }

    /// Pushes reentrant-call arguments, last first.
    fn push_call_args(&mut self, args: &[Expr], params: &[Type]) -> Result<(), FrontendError> {
        for (arg, param) in args.iter().zip(params).rev() {
            let value = self.expr_value(arg, param)?;
            self.body.push(IrInst::Push {
                width: width_for_type(param)?,
                value,
            });
        }
        Ok(())
    }

    fn cleanup_call_args(&mut self, abi: Abi, params: &[Type]) {
        if matches!(abi, Abi::Reentrant) {
            self.body.push(IrInst::CleanupStack {
                bytes: params.iter().map(Type::size_bytes).sum(),
            });
        }
    }

    /// The type and width a binary operation works at: the left operand's,
    /// unless only the right one has a type of its own (`100 - x`).
    fn operand_type(&self, lhs: &Expr, rhs: &Expr) -> Result<(Type, Width), FrontendError> {
        let lhs_ty = self.infer_expr(lhs)?;
        let rhs_ty = self.infer_expr(rhs)?;
        if self.is_const_expr(lhs)? && !self.is_const_expr(rhs)? {
            let width = compatible_expr_width(&rhs_ty, &lhs_ty, true)?;
            return Ok((rhs_ty, width));
        }
        let width = compatible_expr_width(&lhs_ty, &rhs_ty, self.is_const_expr(rhs)?)?;
        Ok((lhs_ty, width))
    }

    /// The signature of the routine pointer variable `name`, if it is one.
    fn routine_pointer(&self, name: &str) -> Option<RoutineType> {
        match &self.bindings.get(name)?.ty {
            Type::Routine(routine) => Some((**routine).clone()),
            _ => None,
        }
    }

    /// For a routine named as a value (not called), the pointer type it has.
    fn routine_address(&self, name: &str) -> Result<Option<RoutineType>, FrontendError> {
        if self.bindings.contains_key(name) {
            return Ok(None);
        }
        match self.parent.routines.get(name) {
            Some(sig) => Ok(Some(routine_type_of(name, sig)?)),
            None => Ok(None),
        }
    }

    fn check_indirect_call(
        &self,
        name: &str,
        routine: &RoutineType,
        args: &[Expr],
        statement: bool,
    ) -> Result<(), FrontendError> {
        if args.len() != routine.params.len() {
            return Err(FrontendError::Type(format!(
                "routine pointer {name} expected {} arguments, got {}",
                routine.params.len(),
                args.len()
            )));
        }
        for (arg, param) in args.iter().zip(&routine.params) {
            if self.is_const_expr(arg)? && integer_width(param).is_some() {
                const_for_type(self.const_expr_value(arg)?, param)?;
            } else {
                ensure_assignable(param, &self.infer_expr(arg)?)?;
            }
        }
        match (&routine.result, statement) {
            (None, false) => Err(FrontendError::Type(format!(
                "procedure pointer {name} cannot be used as an expression"
            ))),
            (Some(_), true) => Err(FrontendError::Type(format!(
                "function pointer {name} result cannot be discarded"
            ))),
            _ => Ok(()),
        }
    }

    /// Calls through the routine pointer variable `name`. `result` is the
    /// expected type and destination of a function's result.
    fn lower_indirect_call(
        &mut self,
        name: &str,
        routine: &RoutineType,
        args: &[Expr],
        result: Option<(&Type, Value)>,
    ) -> Result<(), FrontendError> {
        self.check_indirect_call(name, routine, args, result.is_none())?;
        let target = self.binding(name)?.value.clone();
        if matches!(routine.abi, Abi::Reentrant) {
            self.push_call_args(args, &routine.params)?;
        }
        let Some((expected, dst)) = result else {
            self.body.push(IrInst::CallIndirect {
                target,
                abi: routine.abi,
                result: None,
            });
            self.cleanup_call_args(routine.abi, &routine.params);
            return Ok(());
        };
        let result_ty = routine.result.clone().expect("checked: a function");
        ensure_assignable(expected, &result_ty)?;
        let width = width_for_type(&result_ty)?;
        // As for direct calls: the 65816 cleanup clobbers A, so a reentrant
        // result is parked in X across it.
        let capture_before_cleanup = matches!(routine.abi, Abi::Reentrant)
            && (matches!(dst, Value::Accumulator | Value::Stack { .. })
                || (matches!(self.routine.abi, Abi::Reentrant)
                    && matches!(dst, Value::Temp(_) | Value::Local(_) | Value::Param(_))));
        let call_dst = if capture_before_cleanup {
            Value::IndexX
        } else {
            dst.clone()
        };
        self.body.push(IrInst::CallIndirect {
            target,
            abi: routine.abi,
            result: Some((width, call_dst.clone())),
        });
        self.cleanup_call_args(routine.abi, &routine.params);
        if capture_before_cleanup {
            self.body.push(IrInst::Move {
                width,
                dst,
                src: call_dst,
            });
        }
        Ok(())
    }

    /// `*`, `/`, `%` and `xor`. Multiplying and dividing by powers of two
    /// becomes shifts and masks.
    fn lower_binary(
        &mut self,
        expr: &Expr,
        lhs: &Expr,
        rhs: &Expr,
        expected: &Type,
        dst: Value,
    ) -> Result<(), FrontendError> {
        let (lhs_ty, width) = self.operand_type(lhs, rhs)?;
        ensure_assignable(expected, &lhs_ty)?;
        let signed = lhs_ty.is_signed();
        let constant = if self.is_const_expr(rhs)? {
            Some(self.const_expr_value(rhs)?)
        } else {
            None
        };
        let lhs = self.expr_value(lhs, &lhs_ty)?;
        let shift = constant.and_then(is_power_of_two);
        let op = match expr {
            Expr::Mul(..) => BinaryOp::Mul,
            Expr::Div(..) => BinaryOp::Div { signed },
            Expr::Rem(..) => BinaryOp::Rem { signed },
            Expr::BitXor(..) => BinaryOp::Xor,
            _ => unreachable!("lower_binary handles multiplicative operators"),
        };
        if constant == Some(0) && matches!(op, BinaryOp::Div { .. } | BinaryOp::Rem { .. }) {
            return Err(FrontendError::Type("division by constant zero".to_owned()));
        }
        match (op, shift) {
            (BinaryOp::Mul, Some(0)) | (BinaryOp::Div { .. }, Some(0)) => {
                self.body.push(IrInst::Move {
                    width,
                    dst,
                    src: lhs,
                });
                return Ok(());
            }
            (BinaryOp::Mul, Some(count)) if u16::from(count) < width.size_bytes() * 8 => {
                self.body.push(IrInst::ShiftLeft {
                    width,
                    dst,
                    lhs,
                    rhs: Value::Const(Const::U8(count)),
                });
                return Ok(());
            }
            (BinaryOp::Div { signed: false }, Some(count))
                if u16::from(count) < width.size_bytes() * 8 =>
            {
                self.body.push(IrInst::ShiftRight {
                    width,
                    dst,
                    lhs,
                    rhs: Value::Const(Const::U8(count)),
                });
                return Ok(());
            }
            (BinaryOp::Rem { signed: false }, Some(_)) => {
                let mask = constant.expect("power of two") - 1;
                self.body.push(IrInst::BitAnd {
                    width,
                    dst,
                    lhs,
                    rhs: Value::Const(wrap_const(mask, width)),
                });
                return Ok(());
            }
            _ => {}
        }
        let rhs = self.expr_value(rhs, &lhs_ty)?;
        self.body.push(IrInst::Binary {
            op,
            width,
            dst,
            lhs,
            rhs,
        });
        Ok(())
    }

    fn result_constructor_const(
        &self,
        value: &Expr,
        payload_ty: &Type,
        tag: u8,
    ) -> Result<u16, FrontendError> {
        let Expr::Number(payload) = value else {
            return Err(FrontendError::Type(
                "result constructors currently require constant payloads".to_owned(),
            ));
        };
        if payload_ty.size_bytes() != 1 {
            return Err(FrontendError::Type(
                "result constructors currently support byte payloads".to_owned(),
            ));
        }
        if *payload > u8::MAX as u32 {
            return Err(FrontendError::Type(format!(
                "result payload ${payload:x} does not fit in one byte"
            )));
        }
        Ok(((*payload as u16) << 8) | tag as u16)
    }

    fn binding(&self, name: &str) -> Result<&Binding, FrontendError> {
        self.bindings
            .get(name)
            .ok_or_else(|| FrontendError::Type(format!("unknown variable {name}")))
    }

    /// `addr(name)`: the address of a global variable, as a near byte pointer.
    fn addr_builtin_symbol(&self, args: &[Expr]) -> Result<String, FrontendError> {
        let [Expr::Name(name)] = args else {
            return Err(FrontendError::Type(
                "addr expects one global variable name".to_owned(),
            ));
        };
        match &self.binding(name)?.value {
            Value::Global(symbol) => Ok(symbol.clone()),
            _ => Err(FrontendError::Type(format!(
                "addr({name}) needs a global variable"
            ))),
        }
    }

    /// Shift counts must be constants smaller than the shifted value's width.
    fn shift_count(&self, lhs_ty: &Type, rhs: &Expr) -> Result<u8, FrontendError> {
        let Some(bytes) = integer_width(lhs_ty) else {
            return Err(FrontendError::Type(format!("cannot shift a {lhs_ty:?}")));
        };
        let count = self
            .const_expr_value(rhs)
            .map_err(|_| FrontendError::Type("shift counts must be constants".to_owned()))?;
        if count < 0 || count >= i64::from(bytes) * 8 {
            return Err(FrontendError::Type(format!(
                "shift count {count} is not smaller than the {}-bit operand",
                bytes * 8
            )));
        }
        Ok(count as u8)
    }

    /// Whether `expr` is a compile-time constant: numbers, named constants,
    /// and arithmetic on them.
    fn is_const_expr(&self, expr: &Expr) -> Result<bool, FrontendError> {
        Ok(is_const(expr, &|name| self.const_binding(name)))
    }

    /// Evaluates a constant expression (with 64-bit intermediate results).
    fn const_expr_value(&self, expr: &Expr) -> Result<i64, FrontendError> {
        const_eval(expr, &|name| self.const_binding(name))
    }

    fn const_binding(&self, name: &str) -> Option<i64> {
        let binding = self.bindings.get(name)?;
        match binding.value {
            Value::Const(value) => Some(const_value(value, &binding.ty)),
            _ => None,
        }
    }
}

impl RoutineSig {
    fn result_type(&self, name: &str) -> Result<Type, FrontendError> {
        match &self.kind {
            RoutineDeclKind::Procedure => Err(FrontendError::Type(format!(
                "procedure {name} cannot be used as an expression"
            ))),
            RoutineDeclKind::Function { result } => Ok(result.clone()),
        }
    }
}

/// Names ca65 reads as registers (65816 and 45GS02), which cannot be
/// module-level symbols.
const ASSEMBLER_RESERVED: [&str; 7] = ["a", "x", "y", "z", "s", "q", "sp"];

fn check_symbol_name(kind: &str, name: &str) -> Result<(), FrontendError> {
    if ASSEMBLER_RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        Err(FrontendError::Type(format!(
            "{kind} name `{name}` is reserved by the assembler (a register name)"
        )))
    } else {
        Ok(())
    }
}

fn insert_binding(
    bindings: &mut BTreeMap<String, Binding>,
    name: &str,
    binding: Binding,
) -> Result<(), FrontendError> {
    if bindings.insert(name.to_owned(), binding).is_some() {
        Err(FrontendError::Type(format!("duplicate variable {name}")))
    } else {
        Ok(())
    }
}

fn ensure_assignable(expected: &Type, actual: &Type) -> Result<(), FrontendError> {
    if matches!((expected, actual), (Type::Routine(_), Type::Routine(_))) && expected != actual {
        return Err(FrontendError::Type(format!(
            "cannot assign {actual:?} to {expected:?}: the routine signatures differ"
        )));
    }
    if expected == actual
        || (integer_width(expected).is_some() && integer_width(expected) == integer_width(actual))
        || result_constructor_matches(expected, actual)
    {
        Ok(())
    } else {
        Err(FrontendError::Type(format!(
            "cannot assign {actual:?} to {expected:?}"
        )))
    }
}

fn result_constructor_matches(expected: &Type, actual: &Type) -> bool {
    match (expected, actual) {
        (
            Type::Result {
                ok: expected_ok,
                err: _,
            },
            Type::Result { ok, err },
        ) if **err == Type::U8 => **expected_ok == **ok,
        (
            Type::Result {
                ok: _,
                err: expected_err,
            },
            Type::Result { ok, err },
        ) if **ok == Type::U8 => **expected_err == **err,
        _ => false,
    }
}

fn record_field<'a>(ty: &'a Type, name: &str) -> Result<&'a Field, FrontendError> {
    let Type::Record { fields } = ty else {
        return Err(FrontendError::Type(
            "field access needs a record".to_owned(),
        ));
    };
    fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| FrontendError::Type(format!("record has no field {name}")))
}

fn addr_builtin_type() -> Type {
    Type::near_ptr(Type::U8)
}

/// `trap(family, call, arg0, arg1)` makes a numbered system call; `cop_call`
/// is its original, 65816-flavoured name.
fn is_trap_builtin(name: &str) -> bool {
    name == "trap" || name == "cop_call"
}

/// For a vector named `trap_XX`, returns the family number (hex, $00-$0f).
fn trap_vector_family(name: &str) -> Option<Result<u8, String>> {
    let digits = name.strip_prefix("trap_")?;
    Some(
        u8::from_str_radix(digits, 16)
            .ok()
            .filter(|family| *family < TRAP_FAMILIES)
            .ok_or_else(|| {
                format!(
                    "trap vector {name} needs a hex family $00-${:02x}",
                    TRAP_FAMILIES - 1
                )
            }),
    )
}

/// Number of trap families a trap vector table can hold.
pub const TRAP_FAMILIES: u8 = 16;

fn compatible_expr_width(
    lhs: &Type,
    rhs: &Type,
    rhs_is_const: bool,
) -> Result<Width, FrontendError> {
    if integer_width(lhs) == integer_width(rhs) || (rhs_is_const && integer_width(lhs).is_some()) {
        width_for_type(lhs)
    } else {
        Err(FrontendError::Type(format!(
            "incompatible expression types {lhs:?} and {rhs:?}"
        )))
    }
}

fn integer_width(ty: &Type) -> Option<u16> {
    match ty {
        Type::U8 | Type::I8 => Some(1),
        Type::U16 | Type::I16 => Some(2),
        Type::U24 => Some(3),
        Type::U32 | Type::I32 => Some(4),
        Type::Pointer {
            kind: PointerKind::Far,
            ..
        } => Some(4),
        Type::Pointer { .. } | Type::Routine(_) => Some(2),
        Type::Slice { .. } => Some(4),
        Type::Result { ok, err } if ok.size_bytes().max(err.size_bytes()) <= 1 => Some(2),
        Type::Result { ok, err } => Some(ok.size_bytes().max(err.size_bytes()) + 1),
        Type::Record { .. } | Type::Array { .. } => None,
    }
}

fn width_for_type(ty: &Type) -> Result<Width, FrontendError> {
    integer_width(ty)
        .and_then(Width::from_bytes)
        .ok_or_else(|| FrontendError::Type(format!("unsupported type {ty:?}")))
}

fn string_literal_type() -> Type {
    Type::near_ptr(Type::U8)
}

/// Whether `expr` is a constant, given the named constants `lookup` knows.
fn is_const(expr: &Expr, lookup: &dyn Fn(&str) -> Option<i64>) -> bool {
    match expr {
        Expr::Number(_) => true,
        Expr::Name(name) => lookup(name).is_some(),
        Expr::Neg(value) => is_const(value, lookup),
        Expr::Add(lhs, rhs)
        | Expr::Sub(lhs, rhs)
        | Expr::Mul(lhs, rhs)
        | Expr::Div(lhs, rhs)
        | Expr::Rem(lhs, rhs)
        | Expr::BitAnd(lhs, rhs)
        | Expr::BitOr(lhs, rhs)
        | Expr::BitXor(lhs, rhs)
        | Expr::ShiftLeft(lhs, rhs)
        | Expr::ShiftRight(lhs, rhs) => is_const(lhs, lookup) && is_const(rhs, lookup),
        _ => false,
    }
}

/// Evaluates a constant expression with 64-bit intermediate results.
fn const_eval(expr: &Expr, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<i64, FrontendError> {
    let pair = |lhs: &Expr, rhs: &Expr| -> Result<(i64, i64), FrontendError> {
        Ok((const_eval(lhs, lookup)?, const_eval(rhs, lookup)?))
    };
    let value = match expr {
        Expr::Number(value) => i64::from(*value),
        Expr::String(_) => {
            return Err(FrontendError::Type(
                "string literal is not a numeric constant".to_owned(),
            ))
        }
        Expr::Name(name) => {
            lookup(name).ok_or_else(|| FrontendError::Type(format!("{name} is not a constant")))?
        }
        Expr::Neg(value) => -const_eval(value, lookup)?,
        Expr::Add(lhs, rhs) => {
            let (lhs, rhs) = pair(lhs, rhs)?;
            lhs + rhs
        }
        Expr::Sub(lhs, rhs) => {
            let (lhs, rhs) = pair(lhs, rhs)?;
            lhs - rhs
        }
        Expr::Mul(lhs, rhs) => {
            let (lhs, rhs) = pair(lhs, rhs)?;
            lhs.checked_mul(rhs).ok_or_else(|| {
                FrontendError::Type("constant multiplication overflows".to_owned())
            })?
        }
        Expr::Div(lhs, rhs) | Expr::Rem(lhs, rhs) => {
            let (lhs, rhs) = pair(lhs, rhs)?;
            if rhs == 0 {
                return Err(FrontendError::Type("division by constant zero".to_owned()));
            }
            if matches!(expr, Expr::Div(..)) {
                lhs / rhs
            } else {
                lhs % rhs
            }
        }
        Expr::BitAnd(lhs, rhs) => {
            let (lhs, rhs) = pair(lhs, rhs)?;
            lhs & rhs
        }
        Expr::BitOr(lhs, rhs) => {
            let (lhs, rhs) = pair(lhs, rhs)?;
            lhs | rhs
        }
        Expr::BitXor(lhs, rhs) => {
            let (lhs, rhs) = pair(lhs, rhs)?;
            lhs ^ rhs
        }
        Expr::ShiftLeft(lhs, rhs) | Expr::ShiftRight(lhs, rhs) => {
            let (lhs, rhs) = pair(lhs, rhs)?;
            if !(0..32).contains(&rhs) {
                return Err(FrontendError::Type(format!(
                    "constant shift count {rhs} is out of range"
                )));
            }
            if matches!(expr, Expr::ShiftLeft(..)) {
                lhs << rhs
            } else {
                lhs >> rhs
            }
        }
        _ => {
            return Err(FrontendError::Type(
                "expected constant expression".to_owned(),
            ))
        }
    };
    if !(-(1i64 << 40)..(1i64 << 40)).contains(&value) {
        return Err(FrontendError::Type(format!(
            "constant expression value {value} is out of range"
        )));
    }
    Ok(value)
}

/// The narrowest type for a constant: unsigned unless it is negative.
fn const_type(value: i64) -> Type {
    match value {
        -0x80..=-1 => Type::I8,
        -0x8000..=-0x81 => Type::I16,
        _ if value < 0 => Type::I32,
        0..=0xff => Type::U8,
        0x100..=0xffff => Type::U16,
        0x1_0000..=0xff_ffff => Type::U24,
        _ => Type::U32,
    }
}

/// Encodes `value` in `ty`'s width. Negative values are stored in two's
/// complement; anything from the most negative signed value to the largest
/// unsigned value of the width fits.
fn const_for_type(value: i64, ty: &Type) -> Result<Const, FrontendError> {
    let width = width_for_type(ty)?;
    let bits = u32::from(width.size_bytes()) * 8;
    let fits = value >= -(1i64 << (bits - 1)) && value < (1i64 << bits);
    if !fits {
        return Err(FrontendError::Type(if value < 0 {
            format!("constant {value} does not fit in {ty:?}")
        } else {
            format!("constant ${value:x} does not fit in {ty:?}")
        }));
    }
    Ok(wrap_const(value, width))
}

/// `value` truncated to `width` (an explicit cast).
fn wrap_const(value: i64, width: Width) -> Const {
    let raw = value as u64 as u32;
    match width {
        Width::Byte => Const::U8(raw as u8),
        Width::Word => Const::U16(raw as u16),
        Width::Long => Const::U24(raw & 0x00ff_ffff),
        Width::Quad => Const::U32(raw),
    }
}

/// The numeric value of a constant of type `ty`.
fn const_value(value: Const, ty: &Type) -> i64 {
    let bits = value.bits();
    let size = u32::from(value.width().size_bytes()) * 8;
    if ty.is_signed() && size < 64 && bits >> (size - 1) & 1 == 1 {
        i64::from(bits) - (1i64 << size)
    } else {
        i64::from(bits)
    }
}

/// The routine-pointer type that can point at a routine with `sig`, or why
/// it cannot be called indirectly.
fn routine_type_of(name: &str, sig: &RoutineSig) -> Result<RoutineType, FrontendError> {
    match sig.abi {
        Abi::Reentrant => {}
        Abi::Internal | Abi::Exported if sig.params.is_empty() => {}
        Abi::Internal | Abi::Exported => {
            return Err(FrontendError::Type(format!(
                "routine {name} cannot be called through a pointer: its parameters live in its own storage (declare it reentrant)"
            )))
        }
        Abi::Far | Abi::Interrupt => {
            return Err(FrontendError::Type(format!(
                "{:?} routine {name} cannot be called through a pointer",
                sig.abi
            )))
        }
    }
    Ok(RoutineType {
        abi: if sig.params.is_empty() {
            Abi::Internal
        } else {
            Abi::Reentrant
        },
        params: sig.params.iter().map(|param| param.ty.clone()).collect(),
        result: match &sig.kind {
            RoutineDeclKind::Procedure => None,
            RoutineDeclKind::Function { result } => Some(result.clone()),
        },
    })
}

/// Where an element, field or pointer target lives, before it becomes an
/// IR address: a constant offset into something, plus a dynamic 16-bit
/// byte offset where there is one.
enum Place {
    /// Inside a global (`whole` bytes long).
    Global {
        symbol: String,
        offset: u16,
        index: Option<Value>,
        whole: u16,
    },
    /// Inside a local or parameter (constant offsets only).
    Value { base: Value, offset: u16 },
    /// Through a near or far pointer.
    Pointer {
        pointer: Value,
        far: bool,
        offset: u16,
        index: Option<Value>,
    },
}

impl Place {
    fn add_offset(&mut self, more: u16) -> Result<(), FrontendError> {
        let offset = match self {
            Place::Global { offset, .. }
            | Place::Value { offset, .. }
            | Place::Pointer { offset, .. } => offset,
        };
        *offset = offset
            .checked_add(more)
            .ok_or_else(|| FrontendError::Type("offset beyond 64 KiB".to_owned()))?;
        Ok(())
    }
}

/// The variable an element or field expression is part of (not through a
/// pointer).
fn place_root(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(name) => Some(name),
        Expr::Field(base, _) | Expr::Index(base, _) => place_root(base),
        _ => None,
    }
}

/// Appends literal bytes, merging with a preceding run.
fn push_data_bytes(out: &mut Vec<DataItem>, bytes: &[u8]) {
    if let Some(DataItem::Bytes(run)) = out.last_mut() {
        run.extend_from_slice(bytes);
    } else {
        out.push(DataItem::Bytes(bytes.to_vec()));
    }
}

fn is_power_of_two(value: i64) -> Option<u8> {
    (value > 0 && value & (value - 1) == 0).then(|| value.trailing_zeros() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_module_with_procedure_function_variables_and_control_flow() {
        let source = "
            module Demo

            function one(): u8
                return 1
            end

            export far procedure main()
                direct var x: u8
                x = one()
                if x != 0
                    x = x + 1
                else
                    x = 1
                end
                while x != 3
                    x = x + 1
                end
                return
            end
        ";

        let module = parse_source(source).unwrap();

        assert_eq!(module.name, "Demo");
        assert_eq!(module.routines.len(), 2);
        assert_eq!(module.routines[1].locals[0].storage, Storage::Direct);
        let Stmt::If { else_body, .. } = &module.routines[1].body[1] else {
            panic!("expected if statement");
        };
        assert_eq!(else_body.len(), 1);
    }

    #[test]
    fn compiles_if_else_to_branch_around_else_body() {
        let source = "
            module Branches
            export procedure main()
                direct var x: u8
                x = 0
                if x == 0
                    x = 1
                else
                    x = 2
                end
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let body = &module.routines[0].body;

        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Branch { target } if target.0.starts_with("__if_done_")
        )));
        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Label(label) if label.0.starts_with("__if_else_")
        )));
    }

    #[test]
    fn compiles_elsif_as_nested_else_if() {
        let source = "
            module Branches
            export procedure main()
                var x: u8
                var out: u8
                if x == 0
                    out = 1
                elsif x == 1
                    out = 2
                else
                    out = 3
                end
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let body = &module.routines[0].body;

        assert!(
            body.iter()
                .filter(
                    |inst| matches!(inst, IrInst::Label(label) if label.0.starts_with("__if_else_"))
                )
                .count()
                >= 2
        );
    }

    #[test]
    fn compiles_repeat_until_loop() {
        let source = "
            module Loops
            export procedure main()
                var x: u8
                repeat
                    x = x + 1
                until x >= 3
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let body = &module.routines[0].body;

        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Label(label) if label.0.starts_with("__repeat_start_")
        )));
        assert!(body
            .iter()
            .any(|inst| matches!(inst, IrInst::BranchIfCarryClear { .. })));
    }

    #[test]
    fn compiles_named_record_field_access() {
        let source = "
            module Records
            type Device = record
                id: u16
                flags: u8
            end
            var device: Device

            export procedure main()
                device.id = $1234
                device.flags = $80
                return
            end
        ";

        let module = compile_source(source).unwrap();
        assert_eq!(module.globals[0].ty.size_bytes(), 3);
        let body = &module.routines[0].body;

        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Store {
                addr: Address::ValueOffset {
                    base: Value::Global(name),
                    offset: 0
                },
                ..
            } if name == "device"
        )));
        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Store {
                addr: Address::ValueOffset {
                    base: Value::Global(name),
                    offset: 2
                },
                ..
            } if name == "device"
        )));
    }

    #[test]
    fn compiles_and_or_conditions_with_short_circuit_branches() {
        let source = "
            module Conditions
            export procedure main()
                var a: u8
                var b: u8
                var out: u8
                if a == 1 and b != 2 or out >= 3
                    out = 1
                else
                    out = 0
                end
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let body = &module.routines[0].body;

        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Label(label) if label.0.starts_with("__cond_true_")
        )));
        assert!(
            body.iter()
                .filter(|inst| matches!(
                    inst,
                    IrInst::BranchIfEqual { .. }
                        | IrInst::BranchIfNotEqual { .. }
                        | IrInst::BranchIfCarrySet { .. }
                        | IrInst::BranchIfCarryClear { .. }
                ))
                .count()
                >= 3
        );
    }

    #[test]
    fn parses_character_literals_as_byte_constants() {
        let source = "
            module Chars
            export procedure main()
                direct var letter: u8
                letter = 'H' & $ff
                letter = '\\n'
                letter = '\\x1b'
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let body = &module.routines[0].body;

        // Constant expressions fold at compile time.
        assert!(body.contains(&IrInst::LoadConst {
            dst: Value::Local("letter".to_owned()),
            value: Const::U8(b'H'),
        }));
        assert!(body.contains(&IrInst::LoadConst {
            dst: Value::Local("letter".to_owned()),
            value: Const::U8(b'\n'),
        }));
        assert!(body.contains(&IrInst::LoadConst {
            dst: Value::Local("letter".to_owned()),
            value: Const::U8(0x1b),
        }));
    }

    #[test]
    fn compiles_minimal_source_to_ir() {
        let source = "
            module Demo
            const START = $1300
            const ONE = 1
            export procedure main()
                direct var p: near ^u8
                direct var x: u8
                p = START
                x = ONE
                x = x - ONE
                while x != 4
                    x = x + 1
                end
                p^ = x
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let main = &module.routines[0];

        assert_eq!(parse_source(source).unwrap().consts.len(), 2);
        assert!(main.export);
        assert!(matches!(main.abi, Abi::Internal));
        assert!(main.body.iter().any(|inst| matches!(
            inst,
            IrInst::Store {
                addr: Address::NearPtr(Value::Local(name)),
                ..
            } if name == "p"
        )));
        assert!(main.body.iter().any(|inst| matches!(
            inst,
            IrInst::Sub {
                dst: Value::Local(name),
                rhs: Value::Const(Const::U8(1)),
                ..
            } if name == "x"
        )));
    }

    #[test]
    fn compiles_global_array_indexing_to_symbol_indexed_access() {
        let source = "
            module Arrays
            var buffer: [4]u8
            var head: u16
            var out: u8

            export procedure main()
                buffer[0] = '?'
                head = head + 1 & 3
                buffer[head] = '!'
                head = head + 1 & 3
                out = buffer[head]
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let body = &module.routines[0].body;

        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Store {
                addr: Address::Indexed { base, .. },
                ..
            } if base == "buffer"
        )));
        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Load {
                addr: Address::Indexed { base, .. },
                ..
            } if base == "buffer"
        )));
    }

    #[test]
    fn rejects_pointer_integer_size_mismatch() {
        let source = "
            module Bad
            procedure main()
                direct var p: near ^u8
                p = $010000
                return
            end
        ";

        assert!(compile_source(source).is_err());
    }

    #[test]
    fn rejects_unsupported_parameter_types() {
        let source = "
            module Params
            type Pair = record(a: u8, b: u8)
            procedure store(value: Pair)
                return
            end
        ";

        assert!(compile_source(source).is_err());
    }

    #[test]
    fn compiles_os_level_declarations_to_ca65() {
        let source = "
            module Kernel
            metadata platform = \"c256\"
            metadata code_segment = \"BANK1\"
            vector irq = vblank
            fixed($040005) volatile var io: u8
            export bank(2) var page: u16

            interrupt procedure vblank()
                asm \"nop\"
                return
            end
        ";

        let module = compile_source(source).unwrap();
        assert_eq!(module.metadata[0].key, "platform");
        assert_eq!(module.vectors[0].routine, "vblank");
        assert!(module.globals[0].volatile);
        assert_eq!(
            module.globals[0].placement,
            GlobalPlacement::Fixed(0x040005)
        );
        assert_eq!(module.globals[1].placement, GlobalPlacement::Bank(2));

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("io = $040005\n"));
        assert!(asm.contains(".segment \"BANK2\"\n"));
        assert!(asm.contains(".export page\n"));
        assert!(asm.contains(".segment \"BANK1\"\n\n.proc vblank\n"));
        assert!(asm.contains("__vector_irq: .addr vblank\n"));
        assert!(asm.contains(".segment \"MENSCH_META\"\n"));
        assert!(asm.contains("\t.asciiz \"platform=c256\"\n"));
        assert!(asm.contains("\t.asciiz \"code_segment=BANK1\"\n"));
        assert!(asm.contains("\tnop\n"));
        assert!(asm.contains("\tphp\n"));
        assert!(asm.contains("\tphb\n"));
        assert!(asm.contains("\tpha\n"));
        assert!(asm.contains("\tphx\n"));
        assert!(asm.contains("\tphy\n"));
        assert!(asm.contains("\tply\n"));
        assert!(asm.contains("\tplx\n"));
        assert!(asm.contains("\tpla\n"));
        assert!(asm.contains("\tplb\n"));
        assert!(asm.contains("\tplp\n"));
        assert!(asm.contains("\trti\n"));
    }

    #[test]
    fn parses_interrupt_preserve_attribute() {
        let source = "
            module Kernel
            vector irq = irq

            interrupt(preserve = a, x) procedure irq()
                return
            end
        ";

        let module = compile_source(source).unwrap();
        assert_eq!(
            module.routines[0].interrupt_preserve,
            InterruptPreserve {
                a: true,
                x: true,
                y: false,
                p: false,
                dbr: false
            }
        );

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("\tpha\n"));
        assert!(asm.contains("\tphx\n"));
        assert!(!asm.contains("\tphy\n"));
        assert!(!asm.contains("\tphp\n"));
        assert!(!asm.contains("\tphb\n"));
        assert!(asm.contains("\tplx\n"));
        assert!(asm.contains("\tpla\n"));
    }

    #[test]
    fn compiles_cop_interrupt_signature_builtin() {
        let source = "
            module Kernel
            vector cop = cop_dispatch

            var family: u8
            var call: u16

            interrupt(cop, preserve = p, dbr) procedure cop_dispatch()
                family = cop_signature()
                call = cop_a()
                return
            end
        ";

        let module = compile_source(source).unwrap();
        assert!(module.routines[0].interrupt_cop);
        assert!(module.routines[0]
            .body
            .iter()
            .any(|inst| matches!(inst, IrInst::CopSignature { .. })));

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("__mensch_cop_signature: .res 1\n"));
        assert!(asm.contains("__mensch_cop_a: .res 2\n"));
        assert!(asm.contains("__cop_dispatch_cop_signature_load:\n"));
        assert!(asm.contains("\tsta __mensch_cop_a\n"));
        assert!(asm.contains("\tstx __mensch_cop_x\n"));
        assert!(asm.contains("\tsty __mensch_cop_y\n"));
        assert!(asm.contains("\tlda $04,s\n"));
        assert!(asm.contains("\tdec a\n"));
        assert!(asm.contains("\tsta __cop_dispatch_cop_signature_load+1\n"));
        assert!(asm.contains("\tlda $06,s\n"));
        assert!(asm.contains("\tsta __cop_dispatch_cop_signature_load+3\n"));
        assert!(asm.contains("\tlda f:$000000\n"));
        assert!(asm.contains("\tsta __mensch_cop_signature\n"));
    }

    #[test]
    fn parses_interrupt_preserve_dbr_attribute() {
        let source = "
            module Kernel
            vector irq = irq

            interrupt(preserve = dbr) procedure irq()
                return
            end
        ";

        let module = compile_source(source).unwrap();
        assert_eq!(
            module.routines[0].interrupt_preserve,
            InterruptPreserve {
                a: false,
                x: false,
                y: false,
                p: false,
                dbr: true
            }
        );

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("\tphb\n"));
        assert!(asm.contains("\tplb\n"));
        assert!(!asm.contains("\tpha\n"));
        assert!(!asm.contains("\tphx\n"));
        assert!(!asm.contains("\tphy\n"));
        assert!(!asm.contains("\tphp\n"));
    }

    #[test]
    fn parses_interrupt_preserve_none_attribute() {
        let source = "
            module Kernel
            vector irq = irq

            interrupt(preserve = none) procedure irq()
                return
            end
        ";

        let module = compile_source(source).unwrap();
        assert_eq!(
            module.routines[0].interrupt_preserve,
            InterruptPreserve::NONE
        );

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(!asm.contains("\tpha\n"));
        assert!(!asm.contains("\tphx\n"));
        assert!(!asm.contains("\tphy\n"));
        assert!(!asm.contains("\tphp\n"));
        assert!(!asm.contains("\tphb\n"));
        assert!(asm.contains("\tphk\n"));
        assert!(asm.contains("\tplb\n"));
        assert!(asm.contains("\tsep #$20\n"));
        assert!(asm.contains("\t.i16\n"));
        assert!(asm.contains("\trti\n"));
    }

    #[test]
    fn parses_raw_interrupt_attribute() {
        let source = "
            module Kernel
            vector irq = irq

            interrupt(raw) procedure irq()
                asm \"nop\"
                return
            end
        ";

        let module = compile_source(source).unwrap();
        assert!(module.routines[0].interrupt_raw);
        assert_eq!(
            module.routines[0].interrupt_preserve,
            InterruptPreserve::NONE
        );

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("\tnop\n"));
        assert!(!asm.contains("\tpha\n"));
        assert!(!asm.contains("\tphx\n"));
        assert!(!asm.contains("\tphy\n"));
        assert!(!asm.contains("\tphp\n"));
        assert!(!asm.contains("\tphb\n"));
        assert!(!asm.contains("\tphk\n"));
        assert!(!asm.contains("\tplb\n"));
        assert!(!asm.contains("\tsep #$20\n"));
        assert!(!asm.contains("\t.a8\n"));
        assert!(!asm.contains("\t.i16\n"));
        assert!(asm.contains("\trti\n"));
    }

    #[test]
    fn compiles_external_routine_declarations_for_calls_and_vectors() {
        let source = "
            module UsesExternal
            vector irq = serial_irq
            fixed($009f57) volatile var serial_data: u8

            external interrupt procedure serial_irq()
            external procedure serial_init()
            external procedure serial_write(value: u8)
            external function serial_has_data(): u8
            external far procedure banked_init()

            export procedure main()
                serial_init()
                banked_init()
                serial_write('!')
                if serial_has_data() != 0
                    serial_data = serial_has_data()
                end
                return
            end
        ";

        let module = compile_source(source).unwrap();

        assert_eq!(module.routines.len(), 1);
        assert_eq!(module.routines[0].name, "main");

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains(".import serial_irq\n"));
        assert!(asm.contains(".import serial_init\n"));
        assert!(asm.contains(".import serial_write\n"));
        assert!(asm.contains(".import serial_has_data\n"));
        assert!(asm.contains(".import banked_init\n"));
        assert!(asm.contains(".importzp __dp_serial_write_param_value\n"));
        assert!(asm.contains("__vector_irq: .addr serial_irq\n"));
        assert!(asm.contains("\tjsr serial_init\n"));
        assert!(asm.contains("\tjsl banked_init\n"));
        assert!(asm.contains("\tsta __dp_serial_write_param_value\n"));
        assert!(asm.contains("\tjsr serial_write\n"));
        assert!(asm.contains("\tjsr serial_has_data\n"));
        assert!(!asm.contains(".proc serial_irq\n"));
        assert!(!asm.contains(".proc serial_init\n"));
        assert!(!asm.contains(".proc serial_write\n"));
        assert!(!asm.contains(".proc serial_has_data\n"));
        assert!(!asm.contains(".proc banked_init\n"));
    }

    #[test]
    fn compiles_string_literal_as_near_pointer_static_data() {
        let source = "
            module Strings
            external procedure write_text(text: near ^u8)

            procedure main()
                write_text(\"OK\\r\\n\\x1b\")
                return
            end
        ";

        let module = compile_source(source).unwrap();
        assert_eq!(
            module.static_data[0].items,
            vec![crate::ir::DataItem::Bytes(b"OK\r\n\x1b\0".to_vec())]
        );

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("__str_0:\n"));
        assert!(asm.contains("\t.byte $4f, $4b, $0d, $0a, $1b, $00\n"));
        assert!(asm.contains("\tlda #.loword(__str_0)\n"));
        assert!(asm.contains("\tsta __dp_write_text_param_text\n"));
    }

    #[test]
    fn exports_only_declared_public_routines() {
        let source = "
            module Visibility
            vector irq = irq

            export procedure public_helper()
                return
            end

            procedure private_helper()
                return
            end

            export interrupt procedure irq()
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains(".export public_helper\n"));
        assert!(asm.contains(".export irq\n"));
        assert!(!asm.contains(".export private_helper\n"));
        assert!(asm.contains("\trts\n"));
        assert!(asm.contains("\trti\n"));
    }

    #[test]
    fn compiles_direct_page_parameter_passing() {
        let source = "
            module Params
            fixed($001234) var out: u8

            export far procedure main()
                store('!')
                return
            end

            procedure store(value: u8)
                out = value
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("__dp_arg_0: .res 1\n"));
        assert!(asm.contains("\tsta __dp_arg_0\n"));
        assert!(asm.contains("\tjsr store\n"));
        assert!(asm.contains("\tlda __dp_arg_0\n"));
        assert!(!asm.contains(".exportzp __dp_store_param_value\n"));
    }

    #[test]
    fn exports_public_direct_page_parameters() {
        let source = "
            module PublicParams
            export procedure store(value: u8)
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains(".export store\n"));
        assert!(asm.contains(".exportzp __dp_store_param_value\n"));
    }

    #[test]
    fn rejects_interrupt_calls_to_local_parameterized_routines() {
        let source = "
            module IrqHazard
            vector irq = irq

            procedure serial_write(value: u8)
                return
            end

            interrupt procedure irq()
                serial_write('!')
                return
            end
        ";

        let error = compile_source(source).unwrap_err().to_string();

        assert!(
            error.contains("interrupt routine irq cannot call parameterized routine serial_write")
        );
        assert!(error.contains("direct-page parameters are not interrupt-safe"));
    }

    #[test]
    fn rejects_interrupt_calls_to_external_parameterized_routines() {
        let source = "
            module ExternalIrqHazard
            vector irq = irq
            external procedure serial_write(value: u8)

            interrupt procedure irq()
                serial_write('!')
                return
            end
        ";

        let error = compile_source(source).unwrap_err().to_string();

        assert!(
            error.contains("interrupt routine irq cannot call parameterized routine serial_write")
        );
        assert!(error.contains("direct-page parameters are not interrupt-safe"));
    }

    #[test]
    fn allows_interrupt_calls_to_parameterless_routines() {
        let source = "
            module IrqSafeCall
            vector irq = irq

            procedure acknowledge()
                return
            end

            interrupt procedure irq()
                acknowledge()
                return
            end
        ";

        compile_source(source).unwrap();
    }

    #[test]
    fn allows_interrupt_calls_to_reentrant_parameterized_routines() {
        let source = "
            module ReentrantIrqCall
            vector irq = irq

            reentrant procedure serial_write(value: u8)
                return
            end

            interrupt procedure irq()
                serial_write('!')
                return
            end
        ";

        compile_source(source).unwrap();
    }

    #[test]
    fn allows_interrupt_calls_to_external_reentrant_parameterized_routines() {
        let source = "
            module ExternalReentrantIrqCall
            vector irq = irq
            external reentrant procedure serial_write(value: u8)

            interrupt procedure irq()
                serial_write('!')
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains(".import serial_write\n"));
        assert!(!asm.contains(".importzp __dp_serial_write_param_value\n"));
        assert!(asm.contains("\tpha\n"));
        assert!(asm.contains("\tjsr serial_write\n"));
    }

    #[test]
    fn rejects_vectors_to_non_interrupt_procedures() {
        let source = "
            module BadVector
            vector irq = main
            procedure main()
                return
            end
        ";

        assert!(compile_source(source).is_err());
    }

    #[test]
    fn compiles_application_level_types_and_metadata() {
        let source = "
            module App
            import Bytes as B
            library strings
            application main
            arena var heap: [16]u8
            var point: record(x: u8, y: u16)
            var view: []u8

            procedure main()
                return
            end
        ";

        let module = compile_source(source).unwrap();

        assert_eq!(module.imports[0].module, "Bytes");
        assert_eq!(module.imports[0].alias.as_deref(), Some("B"));
        assert_eq!(module.libraries[0].name, "strings");
        assert_eq!(module.entry.as_ref().unwrap().routine, "main");
        assert!(module.routines[0].abi == Abi::Exported);
        assert_eq!(module.globals[0].ty.size_bytes(), 16);
        assert_eq!(module.globals[1].ty.size_bytes(), 3);
        assert_eq!(module.globals[2].ty.size_bytes(), 4);
        assert!(module
            .metadata
            .iter()
            .any(|metadata| metadata.key == "arena" && metadata.value == "heap:16"));

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("\t.asciiz \"entry.application=main\"\n"));
        assert!(asm.contains("\t.asciiz \"import.B=Bytes\"\n"));
        assert!(asm.contains("\t.asciiz \"library=strings\"\n"));
        assert!(asm.contains("\t.asciiz \"arena=heap:16\"\n"));
    }

    #[test]
    fn rejects_missing_application_entry() {
        let source = "
            module BadEntry
            application main
            procedure other()
                return
            end
        ";

        assert!(compile_source(source).is_err());
    }

    fn compile_error(source: &str) -> String {
        compile_source(source)
            .expect_err("source should be rejected")
            .to_string()
    }

    #[test]
    fn folds_negative_and_compound_constants() {
        let source = "
            module Consts
            const NEG = -2
            const SIZE = 3
            export procedure main()
                var value: i16
                var count: u16
                value = NEG * 100 + 1
                count = SIZE * 8 - 1
                return
            end
        ";
        let module = compile_source(source).unwrap();
        let body = &module.routines[0].body;
        assert!(body.contains(&IrInst::LoadConst {
            dst: Value::Local("value".to_owned()),
            value: Const::U16((-199i16) as u16),
        }));
        assert!(body.contains(&IrInst::LoadConst {
            dst: Value::Local("count".to_owned()),
            value: Const::U16(23),
        }));
    }

    #[test]
    fn strength_reduces_power_of_two_multiplication_and_division() {
        let source = "
            module Reduce
            export procedure main()
                var x: u16
                var y: u16
                y = x * 16
                y = x / 4
                y = x % 8
                y = x * 3
                return
            end
        ";
        let module = compile_source(source).unwrap();
        let body = &module.routines[0].body;
        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::ShiftLeft {
                rhs: Value::Const(Const::U8(4)),
                ..
            }
        )));
        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::ShiftRight {
                rhs: Value::Const(Const::U8(2)),
                ..
            }
        )));
        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::BitAnd {
                rhs: Value::Const(Const::U16(7)),
                ..
            }
        )));
        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Binary {
                op: BinaryOp::Mul,
                ..
            }
        )));
    }

    #[test]
    fn signed_operands_select_signed_compares_and_shifts() {
        let source = "
            module Signed
            export procedure main()
                var x: i16
                var u: u16
                if x < -1
                    x = x >> 1
                end
                if u < 1
                    u = u >> 1
                end
                if x == 0
                    return
                end
                return
            end
        ";
        let module = compile_source(source).unwrap();
        let compares: Vec<bool> = module.routines[0]
            .body
            .iter()
            .filter_map(|inst| match inst {
                IrInst::Compare { signed, .. } => Some(*signed),
                _ => None,
            })
            .collect();
        assert_eq!(compares, vec![true, false, false]);
        assert!(module.routines[0].body.iter().any(|inst| matches!(
            inst,
            IrInst::Binary {
                op: BinaryOp::ShiftRightSigned,
                ..
            }
        )));
    }

    #[test]
    fn rejects_routine_pointers_to_storage_parameter_routines() {
        let error = compile_error(
            "
            module Pointers
            type Handler = reentrant procedure(value: u8)
            procedure direct_params(value: u8)
                return
            end
            export procedure main()
                var handler: Handler
                handler = direct_params
                return
            end
            ",
        );
        assert!(error.contains("declare it reentrant"), "{error}");
    }

    #[test]
    fn rejects_routine_pointers_with_a_different_signature() {
        let error = compile_error(
            "
            module Pointers
            type Handler = reentrant procedure(value: u8)
            reentrant procedure wide(value: u16)
                return
            end
            export procedure main()
                var handler: Handler
                handler = wide
                return
            end
            ",
        );
        assert!(error.contains("signatures differ"), "{error}");
    }

    #[test]
    fn rejects_parameterized_routine_types_that_are_not_reentrant() {
        let error = compile_error(
            "
            module Pointers
            type Handler = procedure(value: u8)
            export procedure main()
                return
            end
            ",
        );
        assert!(error.contains("must be reentrant"), "{error}");
    }

    #[test]
    fn rejects_division_by_constant_zero_and_register_names() {
        let error = compile_error(
            "
            module Zero
            export procedure main()
                var x: u16
                x = x / 0
                return
            end
            ",
        );
        assert!(error.contains("division by constant zero"), "{error}");
        let error = compile_error(
            "
            module Names
            var q: u16
            export procedure main()
                return
            end
            ",
        );
        assert!(error.contains("reserved by the assembler"), "{error}");
    }
}
