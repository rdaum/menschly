use std::collections::BTreeMap;
use std::fmt;

use crate::ir::{
    Abi, Address, Const, Global, GlobalPlacement, IrInst, Label, Local, Metadata, Module, Param,
    PointerKind, Routine, RoutineKind, Storage, Type, Value, Vector, Width,
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
    pub globals: Vec<GlobalDecl>,
    pub vectors: Vec<VectorDecl>,
    pub metadata: Vec<MetadataDecl>,
    pub routines: Vec<RoutineDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalDecl {
    pub name: String,
    pub ty: Type,
    pub placement: GlobalPlacement,
    pub volatile: bool,
    pub export: bool,
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
pub struct RoutineDecl {
    pub name: String,
    pub kind: RoutineDeclKind,
    pub abi: Abi,
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
    Assign { target: Expr, value: Expr },
    Call { name: String, args: Vec<Expr> },
    If { cond: Cond, body: Vec<Stmt> },
    While { cond: Cond, body: Vec<Stmt> },
    Return(Option<Expr>),
    InlineAsm(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cond {
    pub lhs: Expr,
    pub op: CondOp,
    pub rhs: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondOp {
    Eq,
    Ne,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Number(u32),
    Name(String),
    Call { name: String, args: Vec<Expr> },
    Add(Box<Expr>, Box<Expr>),
    Deref(Box<Expr>),
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
    Colon,
    Comma,
    Caret,
    Plus,
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
            ':' => out.push(token(TokenKind::Colon, offset)),
            ',' => out.push(token(TokenKind::Comma, offset)),
            '^' => out.push(token(TokenKind::Caret, offset)),
            '+' => out.push(token(TokenKind::Plus, offset)),
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
                    't' => out.push('\t'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
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
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Result<Self, FrontendError> {
        Ok(Self { tokens, pos: 0 })
    }

    fn parse_module(&mut self) -> Result<SourceModule, FrontendError> {
        self.expect_keyword("module")?;
        let name = self.expect_ident()?;
        let mut globals = Vec::new();
        let mut vectors = Vec::new();
        let mut metadata = Vec::new();
        let mut routines = Vec::new();

        while !self.at_eof() {
            if self.take_keyword("metadata") {
                metadata.push(self.parse_metadata_decl()?);
            } else if self.take_keyword("vector") {
                vectors.push(self.parse_vector_decl()?);
            } else if self.starts_global_decl() {
                globals.push(self.parse_global_decl()?);
            } else {
                routines.push(self.parse_routine()?);
            }
        }

        Ok(SourceModule {
            name,
            globals,
            vectors,
            metadata,
            routines,
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

    fn parse_global_decl(&mut self) -> Result<GlobalDecl, FrontendError> {
        let export = self.take_keyword("export");
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
            export,
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
        let mut abi = Abi::Internal;
        if self.take_keyword("export") {
            abi = Abi::Exported;
        }
        if self.take_keyword("far") {
            abi = Abi::Far;
        } else if self.take_keyword("interrupt") {
            abi = Abi::Interrupt;
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
            params,
            locals,
            body,
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
            _ => Err(FrontendError::Parse(format!("unknown type {name}"))),
        }
    }

    fn parse_stmt(&mut self) -> Result<Stmt, FrontendError> {
        if self.take_keyword("if") {
            let cond = self.parse_cond()?;
            let body = self.parse_block()?;
            return Ok(Stmt::If { cond, body });
        }
        if self.take_keyword("while") {
            let cond = self.parse_cond()?;
            let body = self.parse_block()?;
            return Ok(Stmt::While { cond, body });
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
        let mut body = Vec::new();
        while !self.take_keyword("end") {
            if self.at_eof() {
                return Err(self.parse_error("unterminated block"));
            }
            body.push(self.parse_stmt()?);
        }
        Ok(body)
    }

    fn parse_cond(&mut self) -> Result<Cond, FrontendError> {
        let lhs = self.parse_expr()?;
        let op = if self.take(TokenKind::EqualEqual) {
            CondOp::Eq
        } else if self.take(TokenKind::BangEqual) {
            CondOp::Ne
        } else {
            return Err(self.parse_error("expected == or !="));
        };
        let rhs = self.parse_expr()?;
        Ok(Cond { lhs, op, rhs })
    }

    fn parse_expr(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_postfix()?;
        while self.take(TokenKind::Plus) {
            let rhs = self.parse_postfix()?;
            expr = Expr::Add(Box::new(expr), Box::new(rhs));
        }
        Ok(expr)
    }

    fn parse_postfix(&mut self) -> Result<Expr, FrontendError> {
        let mut expr = self.parse_primary()?;
        while self.take(TokenKind::Caret) {
            expr = Expr::Deref(Box::new(expr));
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, FrontendError> {
        if let Some(value) = self.take_number() {
            return Ok(Expr::Number(value));
        }

        let name = self.expect_ident()?;
        if self.take(TokenKind::LParen) {
            let mut args = Vec::new();
            if self.take(TokenKind::RParen) {
                return Ok(Expr::Call { name, args });
            }
            loop {
                args.push(self.parse_expr()?);
                if self.take(TokenKind::RParen) {
                    break;
                }
                self.expect(TokenKind::Comma)?;
            }
            Ok(Expr::Call { name, args })
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
            || self.is_keyword_at(0, "fixed")
            || self.is_keyword_at(0, "bank")
            || (self.is_keyword_at(0, "export")
                && (self.is_keyword_at(1, "var")
                    || self.is_keyword_at(1, "volatile")
                    || self.is_keyword_at(1, "fixed")
                    || self.is_keyword_at(1, "bank")))
    }

    fn next_is_block_end(&self) -> bool {
        self.is_keyword_at(0, "end")
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

#[derive(Debug, Clone)]
struct RoutineSig {
    kind: RoutineDeclKind,
    params: Vec<Type>,
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
    };

    let mut module = Module::new(source.name);
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

    for routine in &source.routines {
        module
            .routines
            .push(lowerer.lower_routine(routine, &source.globals)?);
    }
    lowerer.validate_vectors(&module)?;
    Ok(module)
}

fn routine_sigs(source: &SourceModule) -> Result<BTreeMap<String, RoutineSig>, FrontendError> {
    let mut out = BTreeMap::new();
    for routine in &source.routines {
        if out
            .insert(
                routine.name.clone(),
                RoutineSig {
                    kind: routine.kind.clone(),
                    params: routine
                        .params
                        .iter()
                        .map(|param| param.ty.clone())
                        .collect(),
                },
            )
            .is_some()
        {
            return Err(FrontendError::Type(format!(
                "duplicate routine {}",
                routine.name
            )));
        }
    }
    Ok(out)
}

impl Lowerer {
    fn lower_routine(
        &mut self,
        source: &RoutineDecl,
        globals: &[GlobalDecl],
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
            abi: source.abi,
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
            body: Vec::new(),
        };

        let mut lowerer = RoutineLowerer::new(self, source, globals)?;
        if !source.params.is_empty() {
            return Err(FrontendError::Type(format!(
                "routine {} declares parameters, but parameter passing is not implemented yet",
                source.name
            )));
        }
        for stmt in &source.body {
            lowerer.lower_stmt(stmt)?;
        }
        routine.body = lowerer.body;
        Ok(routine)
    }

    fn validate_vectors(&self, module: &Module) -> Result<(), FrontendError> {
        for vector in &module.vectors {
            let Some(routine) = module
                .routines
                .iter()
                .find(|routine| routine.name == vector.routine)
            else {
                return Err(FrontendError::Type(format!(
                    "vector {} references unknown routine {}",
                    vector.name, vector.routine
                )));
            };
            if !matches!(routine.abi, Abi::Interrupt) {
                return Err(FrontendError::Type(format!(
                    "vector {} references non-interrupt routine {}",
                    vector.name, vector.routine
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
}

impl<'a> RoutineLowerer<'a> {
    fn new(
        parent: &'a mut Lowerer,
        routine: &'a RoutineDecl,
        globals: &[GlobalDecl],
    ) -> Result<Self, FrontendError> {
        let mut bindings = BTreeMap::new();
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
                self.check_call(name, args, true)?;
                self.body.push(IrInst::CallProcedure { name: name.clone() });
                Ok(())
            }
            Stmt::If { cond, body } => {
                let done = self.parent.label("if_done");
                self.lower_branch_if_false(cond, done.clone())?;
                for stmt in body {
                    self.lower_stmt(stmt)?;
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
            (RoutineDeclKind::Function { result }, Some(value)) => {
                self.lower_expr_to(value, result, Value::Accumulator)?;
                self.body.push(IrInst::ReturnValue(Value::Accumulator));
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
        let lhs_ty = self.infer_expr(&cond.lhs)?;
        let rhs_ty = self.infer_expr(&cond.rhs)?;
        let width = compatible_width(&lhs_ty, &rhs_ty)?;
        self.lower_expr_to(&cond.lhs, &lhs_ty, Value::Accumulator)?;
        let rhs = self.expr_value(&cond.rhs, &lhs_ty)?;
        self.body.push(IrInst::Compare {
            width,
            lhs: Value::Accumulator,
            rhs,
        });
        self.body.push(match cond.op {
            CondOp::Eq => IrInst::BranchIfNotEqual {
                target: false_target,
            },
            CondOp::Ne => IrInst::BranchIfEqual {
                target: false_target,
            },
        });
        Ok(())
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
            Expr::Name(name) => {
                let binding = self.binding(name)?.clone();
                ensure_assignable(expected, &binding.ty)?;
                self.body.push(IrInst::Move {
                    dst,
                    src: binding.value,
                });
                Ok(())
            }
            Expr::Call { name, args } => {
                let result = self.check_call(name, args, false)?;
                ensure_assignable(expected, &result)?;
                self.body.push(IrInst::CallFunction {
                    name: name.clone(),
                    dst,
                });
                Ok(())
            }
            Expr::Add(lhs, rhs) => {
                let lhs_ty = self.infer_expr(lhs)?;
                let rhs_ty = self.infer_expr(rhs)?;
                let width = compatible_width(&lhs_ty, &rhs_ty)?;
                ensure_assignable(expected, &lhs_ty)?;
                self.lower_expr_to(lhs, &lhs_ty, Value::Accumulator)?;
                let rhs = self.expr_value(rhs, &lhs_ty)?;
                self.body.push(IrInst::Add {
                    width,
                    dst,
                    lhs: Value::Accumulator,
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
        }
    }

    fn expr_value(&mut self, expr: &Expr, expected: &Type) -> Result<Value, FrontendError> {
        match expr {
            Expr::Number(value) => Ok(Value::Const(const_for_type(*value, expected)?)),
            Expr::Name(name) => {
                let binding = self.binding(name)?;
                ensure_assignable(expected, &binding.ty)?;
                Ok(binding.value.clone())
            }
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
        let addr = match kind {
            PointerKind::Near | PointerKind::Direct | PointerKind::Code => Address::NearPtr(value),
            PointerKind::Far => Address::FarPtr(value),
        };
        Ok((addr, *to))
    }

    fn infer_expr(&self, expr: &Expr) -> Result<Type, FrontendError> {
        match expr {
            Expr::Number(value) if *value <= 0xff => Ok(Type::U8),
            Expr::Number(value) if *value <= 0xffff => Ok(Type::U16),
            Expr::Number(_) => Ok(Type::U24),
            Expr::Name(name) => Ok(self.binding(name)?.ty.clone()),
            Expr::Call { name, args } => self.check_call(name, args, false),
            Expr::Add(lhs, rhs) => {
                let lhs_ty = self.infer_expr(lhs)?;
                let rhs_ty = self.infer_expr(rhs)?;
                compatible_width(&lhs_ty, &rhs_ty)?;
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
        }
    }

    fn check_call(
        &self,
        name: &str,
        args: &[Expr],
        statement: bool,
    ) -> Result<Type, FrontendError> {
        let sig = self
            .parent
            .routines
            .get(name)
            .ok_or_else(|| FrontendError::Type(format!("unknown routine {name}")))?;
        if args.len() != sig.params.len() {
            return Err(FrontendError::Type(format!(
                "routine {name} expected {} arguments, got {}",
                sig.params.len(),
                args.len()
            )));
        }
        if !args.is_empty() {
            return Err(FrontendError::Type(format!(
                "routine {name} uses parameters, but parameter passing is not implemented yet"
            )));
        }
        for (arg, param_ty) in args.iter().zip(&sig.params) {
            let arg_ty = self.infer_expr(arg)?;
            ensure_assignable(param_ty, &arg_ty)?;
        }

        match (&sig.kind, statement) {
            (RoutineDeclKind::Procedure, true) => Ok(Type::U8),
            (RoutineDeclKind::Procedure, false) => Err(FrontendError::Type(format!(
                "procedure {name} cannot be used as an expression"
            ))),
            (RoutineDeclKind::Function { .. }, true) => Err(FrontendError::Type(format!(
                "function {name} result cannot be discarded"
            ))),
            (RoutineDeclKind::Function { result }, false) => Ok(result.clone()),
        }
    }

    fn binding(&self, name: &str) -> Result<&Binding, FrontendError> {
        self.bindings
            .get(name)
            .ok_or_else(|| FrontendError::Type(format!("unknown variable {name}")))
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
    if expected == actual || integer_width(expected) == integer_width(actual) {
        Ok(())
    } else {
        Err(FrontendError::Type(format!(
            "cannot assign {actual:?} to {expected:?}"
        )))
    }
}

fn compatible_width(lhs: &Type, rhs: &Type) -> Result<Width, FrontendError> {
    if integer_width(lhs) == integer_width(rhs) {
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
    }
}

fn width_for_type(ty: &Type) -> Result<Width, FrontendError> {
    match integer_width(ty) {
        Some(1) => Ok(Width::Byte),
        Some(2) => Ok(Width::Word),
        Some(3) => Ok(Width::Long),
        _ => Err(FrontendError::Type(format!("unsupported type {ty:?}"))),
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

            export procedure main()
                direct var x: u8
                x = one()
                if x != 0
                    x = x + 1
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
    }

    #[test]
    fn compiles_minimal_source_to_ir() {
        let source = "
            module Demo
            export procedure main()
                direct var p: near ^u8
                direct var x: u8
                p = $1300
                x = 1
                while x != 4
                    x = x + 1
                end
                p^ = x
                return
            end
        ";

        let module = compile_source(source).unwrap();
        let main = &module.routines[0];

        assert!(matches!(main.abi, Abi::Exported));
        assert!(main.body.iter().any(|inst| matches!(
            inst,
            IrInst::Store {
                addr: Address::NearPtr(Value::Local(name)),
                ..
            } if name == "p"
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
    fn rejects_parameterized_routines_until_call_abi_exists() {
        let source = "
            module Params
            procedure store(value: u8)
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
        assert!(asm.contains("__vector_irq: .addr vblank\n"));
        assert!(asm.contains(".segment \"MENSCH_META\"\n"));
        assert!(asm.contains("\t.asciiz \"platform=c256\"\n"));
        assert!(asm.contains("\tnop\n"));
        assert!(asm.contains("\trti\n"));
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
}
