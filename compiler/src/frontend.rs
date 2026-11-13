use std::collections::BTreeMap;
use std::fmt;

use crate::ir::{
    Abi, Address, Const, CopRegister, Entry, EntryKind, Field, Global, GlobalPlacement, Import,
    InterruptPreserve, IrInst, Label, Library, Local, Metadata, Module, Param, PointerKind,
    Routine, RoutineKind, StaticData, Storage, Type, Value, Vector, Width,
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
    pub value: u32,
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
    Return(Option<Expr>),
    InlineAsm(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cond {
    Compare(CondCompare),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
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
    Call { name: String, args: Vec<Expr> },
    ResultOk(Box<Expr>),
    ResultErr(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    BitAnd(Box<Expr>, Box<Expr>),
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
    Amp,
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
            '&' => out.push(token(TokenKind::Amp, offset)),
            '<' => {
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    out.push(token(TokenKind::LessEqual, offset));
                } else {
                    out.push(token(TokenKind::Less, offset));
                }
            }
            '>' => {
                if matches!(chars.peek(), Some((_, '='))) {
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
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Result<Self, FrontendError> {
        Ok(Self {
            tokens,
            pos: 0,
            types: BTreeMap::new(),
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
                consts.push(self.parse_const_decl()?);
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

    fn parse_const_decl(&mut self) -> Result<ConstDecl, FrontendError> {
        let name = self.expect_ident()?;
        self.expect(TokenKind::Equal)?;
        let value = self
            .take_number()
            .ok_or_else(|| self.parse_error("expected constant value"))?;
        Ok(ConstDecl {
            name,
            value,
            ty: const_type(value),
        })
    }

    fn parse_external_decl(&mut self) -> Result<ExternalDecl, FrontendError> {
        let (_export, abi, interrupt_preserve, interrupt_raw, interrupt_cop, kind, name, params) =
            self.parse_routine_header()?;
        Ok(ExternalDecl {
            name,
            kind,
            abi,
            interrupt_preserve,
            interrupt_raw,
            interrupt_cop,
            params,
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
        Ok(GlobalDecl {
            name,
            ty,
            placement,
            volatile,
            export: export || arena,
            arena,
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
        let (export, abi, interrupt_preserve, interrupt_raw, interrupt_cop, kind, name, params) =
            self.parse_routine_header()?;

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
            interrupt_preserve,
            interrupt_raw,
            interrupt_cop,
            export,
            params,
            locals,
            body,
        })
    }

    fn parse_routine_header(
        &mut self,
    ) -> Result<
        (
            bool,
            Abi,
            InterruptPreserve,
            bool,
            bool,
            RoutineDeclKind,
            String,
            Vec<VarDecl>,
        ),
        FrontendError,
    > {
        let export = self.take_keyword("export");
        let mut abi = Abi::Internal;
        let mut interrupt_preserve = InterruptPreserve::NONE;
        let mut interrupt_raw = false;
        let mut interrupt_cop = false;
        if self.take_keyword("far") {
            abi = Abi::Far;
        } else if self.take_keyword("interrupt") {
            abi = Abi::Interrupt;
            let interrupt_attr = self.parse_interrupt_attr()?;
            interrupt_preserve = interrupt_attr.preserve;
            interrupt_raw = interrupt_attr.raw;
            interrupt_cop = interrupt_attr.cop;
        } else if self.take_keyword("reentrant") {
            abi = Abi::Reentrant;
        }

        let kind = if self.take_keyword("procedure") {
            RoutineDeclKind::Procedure
        } else if self.take_keyword("function") {
            RoutineDeclKind::Function { result: Type::U8 }
        } else {
            return Err(self.parse_error("expected procedure or function"));
        };

        let name = self.expect_ident()?;
        let params = self.parse_params()?;
        let kind = match kind {
            RoutineDeclKind::Procedure => RoutineDeclKind::Procedure,
            RoutineDeclKind::Function { .. } => {
                self.expect(TokenKind::Colon)?;
                RoutineDeclKind::Function {
                    result: self.parse_type()?,
                }
            }
        };

        Ok((
            export,
            abi,
            interrupt_preserve,
            interrupt_raw,
            interrupt_cop,
            kind,
            name,
            params,
        ))
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

        let name = self.expect_ident()?;
        match name.as_str() {
            "u8" => Ok(Type::U8),
            "i8" => Ok(Type::I8),
            "u16" => Ok(Type::U16),
            "i16" => Ok(Type::I16),
            "u24" => Ok(Type::U24),
            _ => self
                .types
                .get(&name)
                .cloned()
                .ok_or_else(|| FrontendError::Parse(format!("unknown type {name}"))),
        }
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
        let mut cond = self.parse_cond_compare()?;
        while self.take_keyword("and") {
            let rhs = self.parse_cond_compare()?;
            cond = Cond::And(Box::new(cond), Box::new(rhs));
        }
        Ok(cond)
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
            return Err(self.parse_error("expected comparison operator"));
        };
        let rhs = self.parse_expr()?;
        Ok(Cond::Compare(CondCompare { lhs, op, rhs }))
    }

    fn parse_expr(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_add()?;
        while self.take(TokenKind::Amp) {
            let rhs = self.parse_add()?;
            expr = Expr::BitAnd(Box::new(expr), Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_add(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_postfix()?;
        loop {
            if self.take(TokenKind::Plus) {
                let rhs = self.parse_postfix()?;
                expr = Expr::Add(Box::new(expr), Box::new(rhs));
            } else if self.take(TokenKind::Minus) {
                let rhs = self.parse_postfix()?;
                expr = Expr::Sub(Box::new(expr), Box::new(rhs));
            } else {
                break;
            }
        }
        Ok(expr)
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
        if let Some(value) = self.take_number() {
            return Ok(Expr::Number(value));
        }
        if let Some(value) = self.take_string() {
            return Ok(Expr::String(value));
        }

        let name = self.expect_ident()?;
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
        "u16" => Some(Type::U16),
        _ => None,
    }
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
    module.globals = source
        .globals
        .iter()
        .map(|global| Global {
            name: global.name.clone(),
            ty: global.ty.clone(),
            placement: global.placement.clone(),
            volatile: global.volatile,
            export: global.export,
        })
        .collect();
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
                    | Type::Pointer {
                        kind: PointerKind::Near | PointerKind::Direct | PointerKind::Code,
                        ..
                    }
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
            if !matches!(sig.abi, Abi::Interrupt) {
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

    fn intern_string(&mut self, value: &str) -> String {
        let id = self.strings.len();
        let name = format!("__str_{id}");
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        self.strings.push(StaticData {
            name: name.clone(),
            bytes,
        });
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
                let sig = self.check_call(name, args, true)?;
                self.lower_call_args(name, args, &sig.params)?;
                self.body.push(IrInst::CallProcedure {
                    name: name.clone(),
                    abi: sig.abi,
                });
                self.cleanup_call_args(name, &sig.params)?;
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
            Stmt::Return(value) => self.lower_return(value.as_ref()),
        }
    }

    fn lower_assign(&mut self, target: &Expr, value: &Expr) -> Result<(), FrontendError> {
        match target {
            Expr::Name(name) => {
                let binding = self.binding(name)?.clone();
                self.lower_expr_to(value, &binding.ty, binding.value)
            }
            Expr::Deref(ptr) => {
                let (addr, pointee) = self.address_for_pointer(ptr)?;
                self.lower_expr_to(value, &pointee, Value::Accumulator)?;
                self.body.push(IrInst::Store {
                    width: width_for_type(&pointee)?,
                    addr,
                    src: Value::Accumulator,
                });
                Ok(())
            }
            Expr::Index(base, index) => {
                let target = Expr::Index(base.clone(), index.clone());
                let element = self.infer_expr(&target)?;
                let value = self.expr_value(value, &element)?;
                let (addr, element) = self.address_for_index(base, index)?;
                self.body.push(IrInst::Store {
                    width: width_for_type(&element)?,
                    addr,
                    src: value,
                });
                Ok(())
            }
            Expr::Field(base, field) => {
                let (addr, field_ty) = self.address_for_field(base, field)?;
                let value = self.expr_value(value, &field_ty)?;
                self.body.push(IrInst::Store {
                    width: width_for_type(&field_ty)?,
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
        }
    }

    fn lower_compare_branch(
        &mut self,
        cond: &CondCompare,
        target: Label,
        branch_when_true: bool,
    ) -> Result<(), FrontendError> {
        let lhs_ty = self.infer_expr(&cond.lhs)?;
        let rhs_ty = self.infer_expr(&cond.rhs)?;
        let width = compatible_expr_width(&lhs_ty, &rhs_ty, self.is_const_expr(&cond.rhs)?)?;
        let lhs = if matches!(cond.lhs, Expr::Number(_) | Expr::String(_) | Expr::Name(_)) {
            self.expr_value(&cond.lhs, &lhs_ty)?
        } else {
            self.lower_expr_to(&cond.lhs, &lhs_ty, Value::Accumulator)?;
            Value::Accumulator
        };
        let rhs = self.expr_value(&cond.rhs, &lhs_ty)?;
        self.body.push(IrInst::Compare { width, lhs, rhs });
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
        match expr {
            Expr::Number(value) => {
                self.body.push(IrInst::LoadConst {
                    dst,
                    value: const_for_type(*value, expected)?,
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
                let binding = self.binding(name)?.clone();
                if let Value::Const(value) = binding.value {
                    self.body.push(IrInst::LoadConst {
                        dst,
                        value: coerce_const(value, expected)?,
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
                if name == "cop_call" {
                    self.lower_cop_call(args, expected, dst)?;
                    return Ok(());
                }
                let sig = self.check_call(name, args, false)?;
                let result = sig.result_type(name)?;
                ensure_assignable(expected, &result)?;
                self.lower_call_args(name, args, &sig.params)?;
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
                self.cleanup_call_args(name, &sig.params)?;
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
                let lhs_ty = self.infer_expr(lhs)?;
                let rhs_ty = self.infer_expr(rhs)?;
                let width = compatible_expr_width(&lhs_ty, &rhs_ty, self.is_const_expr(rhs)?)?;
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
                let lhs_ty = self.infer_expr(lhs)?;
                let rhs_ty = self.infer_expr(rhs)?;
                let width = compatible_expr_width(&lhs_ty, &rhs_ty, self.is_const_expr(rhs)?)?;
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
            Expr::BitAnd(lhs, rhs) => {
                let lhs_ty = self.infer_expr(lhs)?;
                let rhs_ty = self.infer_expr(rhs)?;
                let width = compatible_expr_width(&lhs_ty, &rhs_ty, self.is_const_expr(rhs)?)?;
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
            Expr::Deref(ptr) => {
                let (addr, pointee) = self.address_for_pointer(ptr)?;
                ensure_assignable(expected, &pointee)?;
                self.body.push(IrInst::Load {
                    width: width_for_type(&pointee)?,
                    dst,
                    addr,
                });
                Ok(())
            }
            Expr::Index(base, index) => {
                let (addr, element) = self.address_for_index(base, index)?;
                ensure_assignable(expected, &element)?;
                self.body.push(IrInst::Load {
                    width: width_for_type(&element)?,
                    dst,
                    addr,
                });
                Ok(())
            }
            Expr::Field(base, field) => {
                let (addr, field_ty) = self.address_for_field(base, field)?;
                ensure_assignable(expected, &field_ty)?;
                self.body.push(IrInst::Load {
                    width: width_for_type(&field_ty)?,
                    dst,
                    addr,
                });
                Ok(())
            }
        }
    }

    fn expr_value(&mut self, expr: &Expr, expected: &Type) -> Result<Value, FrontendError> {
        match expr {
            Expr::Number(value) => Ok(Value::Const(const_for_type(*value, expected)?)),
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
            Expr::Name(name) => {
                let binding = self.binding(name)?;
                if let Value::Const(value) = binding.value {
                    return Ok(Value::Const(coerce_const(value, expected)?));
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

    fn address_for_pointer(&mut self, expr: &Expr) -> Result<(Address, Type), FrontendError> {
        let ty = self.infer_expr(expr)?;
        let Type::Pointer { to, kind } = ty else {
            return Err(FrontendError::Type(
                "dereference needs a pointer".to_owned(),
            ));
        };
        let value = self.expr_value(
            expr,
            &Type::Pointer {
                to: to.clone(),
                kind,
            },
        )?;
        let value = if matches!(
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
                width: width_for_type(&Type::Pointer {
                    to: to.clone(),
                    kind,
                })?,
                dst: temp.clone(),
                src: value,
            });
            temp
        };
        let addr = match kind {
            PointerKind::Near | PointerKind::Direct | PointerKind::Code => Address::NearPtr(value),
            PointerKind::Far => Address::FarPtr(value),
        };
        Ok((addr, *to))
    }

    fn address_for_index(
        &mut self,
        base: &Expr,
        index: &Expr,
    ) -> Result<(Address, Type), FrontendError> {
        let Expr::Name(name) = base else {
            return Err(FrontendError::Type(
                "indexed access currently requires a named array".to_owned(),
            ));
        };
        let binding = self.binding(name)?.clone();
        let Type::Array { element, .. } = binding.ty else {
            return Err(FrontendError::Type(
                "indexed access needs an array".to_owned(),
            ));
        };

        let index_ty = if matches!(index, Expr::Number(_)) {
            Type::U16
        } else {
            self.infer_expr(index)?
        };
        if integer_width(&index_ty) != Some(2) {
            return Err(FrontendError::Type(
                "array indexes currently need a 16-bit scalar".to_owned(),
            ));
        }
        self.lower_expr_to(index, &index_ty, Value::Accumulator)?;
        self.body.push(IrInst::Move {
            width: Width::Word,
            dst: Value::IndexX,
            src: Value::Accumulator,
        });

        match binding.value {
            Value::Global(symbol) => Ok((Address::SymbolIndexedX(symbol), *element)),
            other => Err(FrontendError::Type(format!(
                "indexed access currently requires a global array, got {other:?}"
            ))),
        }
    }

    fn address_for_field(
        &mut self,
        base: &Expr,
        field_name: &str,
    ) -> Result<(Address, Type), FrontendError> {
        let base_ty = self.infer_expr(base)?;
        let field = record_field(&base_ty, field_name)?.clone();
        let Expr::Name(name) = base else {
            return Err(FrontendError::Type(
                "field access currently requires a named record value".to_owned(),
            ));
        };
        let binding = self.binding(name)?.clone();
        Ok((
            Address::ValueOffset {
                base: binding.value,
                offset: field.offset,
            },
            field.ty,
        ))
    }

    fn infer_expr(&self, expr: &Expr) -> Result<Type, FrontendError> {
        match expr {
            Expr::Number(value) if *value <= 0xff => Ok(Type::U8),
            Expr::Number(value) if *value <= 0xffff => Ok(Type::U16),
            Expr::Number(_) => Ok(Type::U24),
            Expr::String(_) => Ok(string_literal_type()),
            Expr::Name(name) => Ok(self.binding(name)?.ty.clone()),
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
            Expr::Call { name, args } if name == "cop_call" => {
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
            Expr::Add(lhs, rhs) | Expr::Sub(lhs, rhs) => {
                let lhs_ty = self.infer_expr(lhs)?;
                let rhs_ty = self.infer_expr(rhs)?;
                compatible_expr_width(&lhs_ty, &rhs_ty, self.is_const_expr(rhs)?)?;
                Ok(lhs_ty)
            }
            Expr::BitAnd(lhs, rhs) => {
                let lhs_ty = self.infer_expr(lhs)?;
                let rhs_ty = self.infer_expr(rhs)?;
                compatible_expr_width(&lhs_ty, &rhs_ty, self.is_const_expr(rhs)?)?;
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
                let index_ty = if matches!(index.as_ref(), Expr::Number(_)) {
                    Type::U16
                } else {
                    self.infer_expr(index)?
                };
                if integer_width(&index_ty) != Some(2) {
                    return Err(FrontendError::Type(
                        "array indexes currently need a 16-bit scalar".to_owned(),
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
        let source_ty = self.infer_expr(&args[0])?;
        let value = self.expr_value(&args[0], &source_ty)?;
        if matches!(target, Type::U16) && matches!(source_ty, Type::U8) {
            self.body.push(IrInst::LoadConst {
                dst: dst.clone(),
                value: Const::U16(0),
            });
            self.body.push(IrInst::Move {
                width: Width::Byte,
                dst,
                src: value,
            });
            return Ok(());
        }
        self.body.push(IrInst::Move {
            width: width_for_type(target)?,
            dst,
            src: value,
        });
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
        if family > u8::MAX as u32 {
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
        self.body.push(IrInst::CopCall {
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
        args: &[Expr],
        params: &[ParamSig],
    ) -> Result<(), FrontendError> {
        let abi = self
            .parent
            .routines
            .get(routine)
            .map(|sig| sig.abi)
            .ok_or_else(|| FrontendError::Type(format!("unknown routine {routine}")))?;

        if matches!(abi, Abi::Reentrant) {
            for (arg, param) in args.iter().zip(params).rev() {
                let value = self.expr_value(arg, &param.ty)?;
                self.body.push(IrInst::Push {
                    width: width_for_type(&param.ty)?,
                    value,
                });
            }
            return Ok(());
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

    fn cleanup_call_args(
        &mut self,
        routine: &str,
        params: &[ParamSig],
    ) -> Result<(), FrontendError> {
        let abi = self
            .parent
            .routines
            .get(routine)
            .map(|sig| sig.abi)
            .ok_or_else(|| FrontendError::Type(format!("unknown routine {routine}")))?;
        if matches!(abi, Abi::Reentrant) {
            self.body.push(IrInst::CleanupStack {
                bytes: params.iter().map(|param| param.ty.size_bytes()).sum(),
            });
        }
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

    fn is_const_expr(&self, expr: &Expr) -> Result<bool, FrontendError> {
        match expr {
            Expr::Number(_) => Ok(true),
            Expr::String(_) => Ok(false),
            Expr::Name(name) => Ok(matches!(self.binding(name)?.value, Value::Const(_))),
            _ => Ok(false),
        }
    }

    fn const_expr_value(&self, expr: &Expr) -> Result<u32, FrontendError> {
        match expr {
            Expr::Number(value) => Ok(*value),
            Expr::String(_) => Err(FrontendError::Type(
                "string literal is not a numeric constant".to_owned(),
            )),
            Expr::Name(name) => {
                let binding = self.binding(name)?;
                match binding.value {
                    Value::Const(value) => const_to_u32(value),
                    _ => Err(FrontendError::Type(format!("{name} is not a constant"))),
                }
            }
            _ => Err(FrontendError::Type(
                "expected constant expression".to_owned(),
            )),
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
        Type::Pointer {
            kind: PointerKind::Far,
            ..
        } => Some(3),
        Type::Pointer { .. } => Some(2),
        Type::Slice { .. } => Some(4),
        Type::Result { ok, err } if ok.size_bytes().max(err.size_bytes()) <= 1 => Some(2),
        Type::Result { ok, err } => Some(ok.size_bytes().max(err.size_bytes()) + 1),
        Type::Record { .. } | Type::Array { .. } => None,
    }
}

fn width_for_type(ty: &Type) -> Result<Width, FrontendError> {
    match integer_width(ty) {
        Some(1) => Ok(Width::Byte),
        Some(2) => Ok(Width::Word),
        Some(3) => Ok(Width::Long),
        Some(4) => Err(FrontendError::Type(format!(
            "type {ty:?} is wider than current scalar operations"
        ))),
        _ => Err(FrontendError::Type(format!("unsupported type {ty:?}"))),
    }
}

fn string_literal_type() -> Type {
    Type::near_ptr(Type::U8)
}

fn const_type(value: u32) -> Type {
    if value <= 0xff {
        Type::U8
    } else if value <= 0xffff {
        Type::U16
    } else {
        Type::U24
    }
}

fn const_for_type(value: u32, ty: &Type) -> Result<Const, FrontendError> {
    match width_for_type(ty)? {
        Width::Byte if value <= 0xff => Ok(Const::U8(value as u8)),
        Width::Word if value <= 0xffff => Ok(Const::U16(value as u16)),
        Width::Long if value <= 0x00ff_ffff => Ok(Const::U24(value)),
        _ => Err(FrontendError::Type(format!(
            "constant ${value:x} does not fit in {ty:?}"
        ))),
    }
}

fn coerce_const(value: Const, ty: &Type) -> Result<Const, FrontendError> {
    const_for_type(const_to_u32(value)?, ty)
}

fn const_to_u32(value: Const) -> Result<u32, FrontendError> {
    Ok(match value {
        Const::U8(value) => u32::from(value),
        Const::U16(value) => u32::from(value),
        Const::U24(value) => value,
    })
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

        assert!(body.iter().any(|inst| matches!(inst, IrInst::BitAnd {
            dst: Value::Local(name),
            lhs: Value::Const(Const::U8(b'H')),
            rhs: Value::Const(Const::U8(0xff)),
            ..
        } if name == "letter")));
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
                addr: Address::SymbolIndexedX(name),
                ..
            } if name == "buffer"
        )));
        assert!(body.iter().any(|inst| matches!(
            inst,
            IrInst::Load {
                addr: Address::SymbolIndexedX(name),
                ..
            } if name == "buffer"
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
            procedure store(value: u24)
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
        assert_eq!(module.static_data[0].bytes, b"OK\r\n\x1b\0");

        let (lowered, _assignment) =
            crate::lower_65816::lower_module_with_storage(&module).unwrap();
        let asm = crate::ca65::emit_module(&lowered);

        assert!(asm.contains("__str_0:\n"));
        assert!(asm.contains("\t.byte $4f, $4b, $0d, $0a, $1b, $00\n"));
        assert!(asm.contains("\tlda #__str_0\n"));
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
}
