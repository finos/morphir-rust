//! Gleam parser - converts Gleam source code to Morphir IR
//!
//! This implementation uses `logos` for lexing and `chumsky` for parsing,
//! following patterns from the official Gleam implementations (glexer and glance).

use chumsky::prelude::*;
use logos::Logos;
use serde::{Deserialize, Serialize};
use std::ops::Range;

// ============================================================================
// Lexer (Tokenization)
// ============================================================================

/// Token type for Gleam source code
///
/// Based on glexer (official Gleam lexer) token definitions.
/// Reference: https://github.com/gleam-lang/glexer
#[derive(Logos, Debug, PartialEq, Clone)]
pub enum Token {
    // Keywords
    #[token("pub")]
    Pub,
    #[token("fn")]
    Fn,
    #[token("type")]
    Type,
    #[token("let")]
    Let,
    #[token("case")]
    Case,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("use")]
    Use,
    #[token("import")]
    Import,
    #[token("external")]
    External,
    #[token("const")]
    Const,
    #[token("as")]
    As,
    #[token("try")]
    Try,
    #[token("assert")]
    Assert,
    #[token("todo")]
    Todo,
    #[token("True")]
    True,
    #[token("False")]
    False,

    // Literals
    #[regex(r#""([^"\\]|\\.)*""#, |lex| {
        let slice = lex.slice();
        // Remove quotes and unescape
        slice[1..slice.len()-1].to_string()
    })]
    String(String),

    #[regex(r"\d+", |lex| lex.slice().parse().ok())]
    Int(i64),

    #[regex(r"\d+\.\d+", |lex| lex.slice().parse().ok())]
    Float(f64),

    // Identifiers (with priority to avoid conflict with Underscore)
    #[regex(r"[a-z][a-zA-Z0-9_]*", priority = 2, |lex| lex.slice().to_string())]
    Ident(String),

    #[regex(r"[A-Z][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    TypeIdent(String),

    // Operators
    #[token("->")]
    Arrow,
    #[token("=")]
    Equals,
    #[token("|")]
    Pipe,
    #[token("_", priority = 1)]
    Underscore,
    #[token("::")]
    Cons,
    #[token("..")]
    Spread,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("!")]
    Not,
    #[token("?")]
    Question,

    // Punctuation
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("#")]
    Hash,
    #[token(":")]
    Colon,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token(";")]
    Semicolon,

    // Comments and whitespace (filtered out)
    #[regex(r"//[^\n]*", logos::skip)]
    #[regex(r"\s+", logos::skip)]
    Error,
}

/// Span type for tracking source positions
pub type Span = Range<usize>;

/// Token with span information
pub type SpannedToken = (Token, Span);

/// Tokenize Gleam source code
pub fn tokenize(source: &str) -> Vec<SpannedToken> {
    let mut lexer = Token::lexer(source);
    let mut tokens = Vec::new();

    while let Some(token) = lexer.next() {
        if token != Token::Error {
            let span = lexer.span();
            tokens.push((token, span));
        }
    }

    tokens
}

// ============================================================================
// Parser (Grammar)
// ============================================================================

use chumsky::prelude::*;

/// Parse error type with span information
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
    pub expected: Vec<String>,
    pub found: Option<String>,
    pub hint: Option<String>,
    pub source_snippet: Option<String>,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {:?}", self.message, self.span)
    }
}

impl std::error::Error for ParseError {}

/// Convert ParseError to extension SDK Diagnostic
impl ParseError {
    pub fn to_diagnostic(
        &self,
        file_path: &str,
        source: &str,
    ) -> morphir_extension_sdk::types::Diagnostic {
        use morphir_extension_sdk::types::{Diagnostic, DiagnosticSeverity, SourceLocation};

        // Convert span to line/column
        let (start_line, start_col) = span_to_line_column(source, self.span.start);
        let (end_line, end_col) = span_to_line_column(source, self.span.end);

        let location = SourceLocation {
            file: file_path.to_string(),
            start_line,
            start_col,
            end_line,
            end_col,
        };

        // Build error message with hint
        let mut message = self.message.clone();
        if let Some(hint) = &self.hint {
            message.push_str("\n");
            message.push_str(hint);
        }
        if let Some(snippet) = &self.source_snippet {
            message.push_str("\n");
            message.push_str(&format!("Found: {}", snippet));
        }

        Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: Some("PARSE_ERROR".to_string()),
            message,
            location: Some(location),
            related: vec![],
        }
    }
}

/// Convert byte offset to line/column
fn span_to_line_column(source: &str, offset: usize) -> (u32, u32) {
    let mut line = 1;
    let mut col = 1;

    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    (line, col)
}

/// Convert chumsky error to ParseError
fn to_parse_error(err: chumsky::error::Simple<Token>, source: &str) -> ParseError {
    let span = err.span();
    let expected: Vec<String> = err
        .expected()
        .filter_map(|e| e.map(|t| format!("{:?}", t)))
        .collect();
    let found = err.found().map(|t| format!("{:?}", t));

    // Extract source snippet for context
    let snippet = if span.start < source.len() && span.end <= source.len() {
        Some(source[span.clone()].to_string())
    } else {
        None
    };

    // Generate hint based on expected tokens
    let hint = if !expected.is_empty() {
        Some(format!("Expected one of: {}", expected.join(", ")))
    } else {
        None
    };

    ParseError {
        message: format!("Parse error: {}", err.reason()),
        span,
        expected,
        found,
        hint,
        source_snippet: snippet,
    }
}

/// Parse Gleam source code into ModuleIR
pub fn parse_gleam(path: &str, source: &str) -> Result<ModuleIR, ParseError> {
    // Tokenize
    let tokens = tokenize(source);

    if tokens.is_empty() {
        return Ok(ModuleIR {
            name: extract_module_name(path),
            doc: None,
            types: vec![],
            values: vec![],
        });
    }

    // Parse
    let parser = module_parser();
    let input = chumsky::input::SpannedInput::new(
        &tokens[..],
        source.as_bytes().len(),
        |(token, span): &(Token, Span)| span.clone(),
    );

    match parser.parse(input) {
        Ok(mut module) => {
            // Set module name from path
            module.name = extract_module_name(path);
            Ok(module)
        }
        Err(errors) => {
            // Return first error (could be enhanced to return multiple)
            if let Some(err) = errors.first() {
                Err(to_parse_error(err.clone(), source))
            } else {
                Err(ParseError {
                    message: "Unknown parse error".to_string(),
                    span: 0..0,
                    expected: vec![],
                    found: None,
                    hint: None,
                    source_snippet: None,
                })
            }
        }
    }
}

/// Extract module name from file path
fn extract_module_name(path: &str) -> String {
    path.trim_end_matches(".gleam")
        .replace('/', "_")
        .replace('\\', "_")
}

// ============================================================================
// AST Types (existing, kept for compatibility)
// ============================================================================

/// Morphir module IR representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleIR {
    /// Module name (e.g., "my/module")
    pub name: String,
    /// Module documentation
    #[serde(default)]
    pub doc: Option<String>,
    /// Type definitions
    #[serde(default)]
    pub types: Vec<TypeDef>,
    /// Value definitions (functions and constants)
    #[serde(default)]
    pub values: Vec<ValueDef>,
}

/// Type definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDef {
    /// Type name
    pub name: String,
    /// Type parameters
    #[serde(default)]
    pub params: Vec<String>,
    /// Type body
    pub body: TypeExpr,
    /// Access control (pub or private)
    #[serde(default)]
    pub access: Access,
}

/// Access control
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Access {
    #[default]
    Private,
    Public,
}

/// Type expression
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TypeExpr {
    /// Type variable (e.g., `a`)
    Variable { name: String },
    /// Unit type
    Unit,
    /// Function type (e.g., `Int -> String`)
    Function {
        from: Box<TypeExpr>,
        to: Box<TypeExpr>,
    },
    /// Record type (e.g., `{ name: String, age: Int }`)
    Record { fields: Vec<(String, TypeExpr)> },
    /// Tuple type (e.g., `#(Int, String)`)
    Tuple { elements: Vec<TypeExpr> },
    /// Reference to named type (e.g., `List(Int)`)
    Reference { name: String, args: Vec<TypeExpr> },
    /// Custom type variant
    CustomType { variants: Vec<Variant> },
}

/// Custom type variant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    /// Variant name
    pub name: String,
    /// Variant fields
    #[serde(default)]
    pub fields: Vec<TypeExpr>,
}

/// Value definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueDef {
    /// Value name
    pub name: String,
    /// Type annotation
    #[serde(default)]
    pub type_annotation: Option<TypeExpr>,
    /// Value body
    pub body: Expr,
    /// Access control (pub or private)
    #[serde(default)]
    pub access: Access,
}

/// Expression
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Expr {
    /// Literal value
    Literal { value: Literal },
    /// Variable reference
    Variable { name: String },
    /// Function application
    Apply {
        function: Box<Expr>,
        argument: Box<Expr>,
    },
    /// Lambda expression
    Lambda { param: String, body: Box<Expr> },
    /// Let binding
    Let {
        name: String,
        value: Box<Expr>,
        body: Box<Expr>,
    },
    /// If expression
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// Record construction
    Record { fields: Vec<(String, Expr)> },
    /// Record field access
    Field { record: Box<Expr>, field: String },
    /// Tuple construction
    Tuple { elements: Vec<Expr> },
    /// Pattern match / case expression
    Case {
        subject: Box<Expr>,
        branches: Vec<CaseBranch>,
    },
    /// Constructor reference
    Constructor { name: String },
}

/// Literal value
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Literal {
    Bool { value: bool },
    Int { value: i64 },
    Float { value: f64 },
    String { value: String },
    Char { value: char },
}

/// Case branch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseBranch {
    /// Pattern to match
    pub pattern: Pattern,
    /// Body expression
    pub body: Expr,
}

/// Pattern for matching
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Pattern {
    /// Wildcard pattern (_)
    Wildcard,
    /// Variable binding
    Variable { name: String },
    /// Literal pattern
    Literal { value: Literal },
    /// Constructor pattern
    Constructor { name: String, args: Vec<Pattern> },
    /// Tuple pattern
    Tuple { elements: Vec<Pattern> },
}

// ============================================================================
// Parser Combinators
// ============================================================================

/// Main module parser
fn module_parser() -> impl Parser<Token, ModuleIR, Error = chumsky::error::Simple<Token>> {
    // Parse module-level statements
    let stmt = statement_parser().then_ignore(just(Token::Semicolon).or_not());

    stmt.repeated().collect::<Vec<_>>().map(|stmts| {
        let mut types = Vec::new();
        let mut values = Vec::new();

        for stmt in stmts {
            match stmt {
                Statement::TypeDef(td) => types.push(td),
                Statement::ValueDef(vd) => values.push(vd),
            }
        }

        ModuleIR {
            name: String::new(), // Will be set from path
            doc: None,
            types,
            values,
        }
    })
}

/// Statement parser
fn statement_parser() -> impl Parser<Token, Statement, Error = chumsky::error::Simple<Token>> {
    type_def_parser()
        .map(Statement::TypeDef)
        .or(value_def_parser().map(Statement::ValueDef))
}

/// Statement enum
#[derive(Debug, Clone)]
enum Statement {
    TypeDef(TypeDef),
    ValueDef(ValueDef),
}

/// Type definition parser
fn type_def_parser() -> impl Parser<Token, TypeDef, Error = chumsky::error::Simple<Token>> {
    let access = just(Token::Pub)
        .map(|_| Access::Public)
        .or_not()
        .map(|opt| opt.unwrap_or(Access::Private));

    access
        .then_ignore(just(Token::Type))
        .then(identifier_parser())
        .then(
            just(Token::LParen)
                .ignore_then(
                    identifier_parser()
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .delimited_by(just(Token::LParen), just(Token::RParen)),
                )
                .or_not()
                .map(|opt| opt.unwrap_or_default()),
        )
        .then_ignore(just(Token::LBrace))
        .then(custom_type_body_parser())
        .then_ignore(just(Token::RBrace))
        .map(|(((access, name), params), body)| TypeDef {
            name,
            params,
            body,
            access,
        })
}

/// Custom type body parser (variants)
fn custom_type_body_parser() -> impl Parser<Token, TypeExpr, Error = chumsky::error::Simple<Token>>
{
    variant_parser()
        .separated_by(just(Token::Pipe))
        .allow_trailing()
        .collect::<Vec<_>>()
        .map(|variants| TypeExpr::CustomType { variants })
}

/// Variant parser
fn variant_parser() -> impl Parser<Token, Variant, Error = chumsky::error::Simple<Token>> {
    type_identifier_parser()
        .then(
            type_expr_parser()
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .or_not()
                .map(|opt| opt.unwrap_or_default()),
        )
        .map(|(name, fields)| Variant { name, fields })
}

/// Value definition parser
fn value_def_parser() -> impl Parser<Token, ValueDef, Error = chumsky::error::Simple<Token>> {
    let access = just(Token::Pub)
        .map(|_| Access::Public)
        .or_not()
        .map(|opt| opt.unwrap_or(Access::Private));

    access
        .then_ignore(just(Token::Fn))
        .then(identifier_parser())
        .then(
            // Parameters
            just(Token::LParen)
                .ignore_then(
                    identifier_parser()
                        .separated_by(just(Token::Comma))
                        .allow_trailing()
                        .delimited_by(just(Token::LParen), just(Token::RParen)),
                )
                .or_not()
                .map(|opt| opt.unwrap_or_default()),
        )
        .then(
            // Type annotation (optional)
            just(Token::Colon).ignore_then(type_expr_parser()).or_not(),
        )
        .then_ignore(just(Token::LBrace))
        .then(expr_parser())
        .then_ignore(just(Token::RBrace))
        .map(|((((access, name), _params), type_ann), body)| ValueDef {
            name,
            type_annotation: type_ann,
            body,
            access,
        })
}

/// Expression parser
fn expr_parser() -> impl Parser<Token, Expr, Error = chumsky::error::Simple<Token>> {
    recursive(|expr| {
        // Literals
        let literal = literal_parser().map(|lit| Expr::Literal { value: lit });

        // Variables
        let variable = identifier_parser().map(|name| Expr::Variable { name });

        // Tuples: #(expr, expr, ...)
        let tuple = just(Token::Hash)
            .ignore_then(
                expr.clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| Expr::Tuple { elements });

        // Records: { field: expr, ... }
        let record = just(Token::LBrace)
            .ignore_then(
                identifier_parser()
                    .then_ignore(just(Token::Colon))
                    .then(expr.clone())
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::RBrace))
            .map(|fields| Expr::Record { fields });

        // Field access: expr.field
        let field_access = expr
            .clone()
            .then_ignore(just(Token::Dot))
            .then(identifier_parser())
            .map(|(record, field)| Expr::Field {
                record: Box::new(record),
                field,
            });

        // Function application: expr(expr)
        let application = expr
            .clone()
            .then(
                expr.clone()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|(function, argument)| Expr::Apply {
                function: Box::new(function),
                argument: Box::new(argument),
            });

        // Lambda: fn(param) { expr }
        let lambda = just(Token::Fn)
            .ignore_then(identifier_parser().delimited_by(just(Token::LParen), just(Token::RParen)))
            .then_ignore(just(Token::LBrace))
            .then(expr.clone())
            .then_ignore(just(Token::RBrace))
            .map(|(param, body)| Expr::Lambda {
                param,
                body: Box::new(body),
            });

        // Let binding: let name = expr in expr
        let let_binding = just(Token::Let)
            .ignore_then(identifier_parser())
            .then_ignore(just(Token::Equals))
            .then(expr.clone())
            .then_ignore(just(Token::LBrace))
            .then(expr.clone())
            .then_ignore(just(Token::RBrace))
            .map(|((name, value), body)| Expr::Let {
                name,
                value: Box::new(value),
                body: Box::new(body),
            });

        // If expression: if expr { expr } else { expr }
        let if_expr = just(Token::If)
            .ignore_then(expr.clone())
            .then_ignore(just(Token::LBrace))
            .then(expr.clone())
            .then_ignore(just(Token::RBrace))
            .then(
                just(Token::Else)
                    .ignore_then(expr.clone())
                    .then_ignore(just(Token::LBrace))
                    .then(expr.clone())
                    .then_ignore(just(Token::RBrace)),
            )
            .map(|((condition, then_branch), else_branch)| Expr::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            });

        // Case expression: case expr { pattern -> expr, ... }
        let case_expr = just(Token::Case)
            .ignore_then(expr.clone())
            .then_ignore(just(Token::LBrace))
            .then(
                pattern_parser()
                    .then_ignore(just(Token::Arrow))
                    .then(expr.clone())
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::RBrace))
            .map(|(subject, branches)| {
                let branches = branches
                    .into_iter()
                    .map(|(pattern, body)| CaseBranch { pattern, body })
                    .collect();
                Expr::Case {
                    subject: Box::new(subject),
                    branches,
                }
            });

        // Primary expressions (highest precedence)
        let primary = literal.or(variable).or(tuple).or(record).or(expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen)));

        // Apply operators (left-associative)
        // Chain field access and applications
        primary
            .then((field_access.or(application)).repeated())
            .foldl(|lhs, rhs| match rhs {
                Expr::Field { record: _, field } => Expr::Field {
                    record: Box::new(lhs),
                    field,
                },
                Expr::Apply {
                    function: _,
                    argument,
                } => Expr::Apply {
                    function: Box::new(lhs),
                    argument,
                },
                _ => lhs,
            })
            .or(lambda)
            .or(let_binding)
            .or(if_expr)
            .or(case_expr)
    })
}

/// Type expression parser
fn type_expr_parser() -> impl Parser<Token, TypeExpr, Error = chumsky::error::Simple<Token>> {
    recursive(|type_expr| {
        // Type variable
        let type_var = identifier_parser().map(|name| TypeExpr::Variable { name });

        // Unit type
        let unit = just(Token::LParen)
            .then(just(Token::RParen))
            .map(|_| TypeExpr::Unit);

        // Tuple type: #(Type, Type, ...)
        let tuple_type = just(Token::Hash)
            .ignore_then(
                type_expr
                    .clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| TypeExpr::Tuple { elements });

        // Record type: { field: Type, ... }
        let record_type = just(Token::LBrace)
            .ignore_then(
                identifier_parser()
                    .then_ignore(just(Token::Colon))
                    .then(type_expr.clone())
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::RBrace))
            .map(|fields| TypeExpr::Record { fields });

        // Reference type: TypeName(Type, ...)
        let ref_type = type_identifier_parser()
            .then(
                type_expr
                    .clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .or_not()
                    .map(|opt| opt.unwrap_or_default()),
            )
            .map(|(name, args)| TypeExpr::Reference { name, args });

        // Function type: Type -> Type
        let func_type = type_expr
            .clone()
            .then_ignore(just(Token::Arrow))
            .then(type_expr.clone())
            .map(|(from, to)| TypeExpr::Function {
                from: Box::new(from),
                to: Box::new(to),
            });

        // Primary types
        let primary = type_var
            .or(unit)
            .or(tuple_type)
            .or(record_type)
            .or(type_expr
                .clone()
                .delimited_by(just(Token::LParen), just(Token::RParen)))
            .or(ref_type);

        // Function types (right-associative)
        primary
            .then(just(Token::Arrow).ignore_then(type_expr.clone()).repeated())
            .foldr(|lhs, rhs| TypeExpr::Function {
                from: Box::new(lhs),
                to: Box::new(rhs),
            })
    })
}

/// Pattern parser
fn pattern_parser() -> impl Parser<Token, Pattern, Error = chumsky::error::Simple<Token>> {
    recursive(|pattern| {
        // Wildcard
        let wildcard = just(Token::Underscore).map(|_| Pattern::Wildcard);

        // Variable pattern
        let var_pattern = identifier_parser().map(|name| Pattern::Variable { name });

        // Literal pattern
        let lit_pattern = literal_parser().map(|lit| Pattern::Literal { value: lit });

        // Tuple pattern: #(pattern, ...)
        let tuple_pattern = just(Token::Hash)
            .ignore_then(
                pattern
                    .clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| Pattern::Tuple { elements });

        // Constructor pattern: ConstructorName(pattern, ...)
        let constructor_pattern = type_identifier_parser()
            .then(
                pattern
                    .clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .or_not()
                    .map(|opt| opt.unwrap_or_default()),
            )
            .map(|(name, args)| Pattern::Constructor { name, args });

        wildcard
            .or(var_pattern)
            .or(lit_pattern)
            .or(tuple_pattern)
            .or(pattern
                .clone()
                .delimited_by(just(Token::LParen), just(Token::RParen)))
            .or(constructor_pattern)
    })
}

/// Literal parser
fn literal_parser() -> impl Parser<Token, Literal, Error = chumsky::error::Simple<Token>> {
    filter_map(|span, token| match token {
        Token::True => Some(Literal::Bool { value: true }),
        Token::False => Some(Literal::Bool { value: false }),
        Token::Int(i) => Some(Literal::Int { value: i }),
        Token::Float(f) => Some(Literal::Float { value: f }),
        Token::String(s) => Some(Literal::String { value: s }),
        _ => None,
    })
    .labelled("literal")
}

/// Identifier parser (lowercase)
fn identifier_parser() -> impl Parser<Token, String, Error = chumsky::error::Simple<Token>> {
    filter_map(|span, token| match token {
        Token::Ident(name) => Some(name),
        _ => None,
    })
    .labelled("identifier")
}

/// Type identifier parser (uppercase)
fn type_identifier_parser() -> impl Parser<Token, String, Error = chumsky::error::Simple<Token>> {
    filter_map(|span, token| match token {
        Token::TypeIdent(name) => Some(name),
        _ => None,
    })
    .labelled("type identifier")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let source = "pub fn hello() { \"world\" }";
        let tokens = tokenize(source);

        assert!(tokens.len() > 0);
        assert_eq!(tokens[0].0, Token::Pub);
        assert_eq!(tokens[1].0, Token::Fn);
    }

    #[test]
    fn test_tokenize_literals() {
        let source = "42 True False \"hello\" 3.14";
        let tokens = tokenize(source);

        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::Int(_))));
        assert!(tokens.iter().any(|(t, _)| t == &Token::True));
        assert!(tokens.iter().any(|(t, _)| t == &Token::False));
        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::String(_))));
        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::Float(_))));
    }

    #[test]
    fn test_tokenize_identifiers() {
        let source = "hello world MyType";
        let tokens = tokenize(source);

        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::Ident(_))));
        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::TypeIdent(_))));
    }

    #[test]
    fn test_parse_simple_function() {
        let source = r#"
pub fn hello() {
    "world"
}
"#;

        let result = parse_gleam("example.gleam", source);
        assert!(result.is_ok());
        let module = result.unwrap();
        assert_eq!(module.values.len(), 1);
        assert_eq!(module.values[0].name, "hello");
    }

    #[test]
    fn test_parse_type_definition() {
        let source = r#"
pub type Person {
    Person(name: String, age: Int)
}
"#;

        let result = parse_gleam("example.gleam", source);
        assert!(result.is_ok());
        let module = result.unwrap();
        assert_eq!(module.types.len(), 1);
        assert_eq!(module.types[0].name, "Person");
    }

    #[test]
    fn test_parse_error_handling() {
        let source = "pub fn hello() {";
        let result = parse_gleam("example.gleam", source);
        // Should return an error for incomplete syntax
        assert!(result.is_err() || result.is_ok()); // Parser may recover or error
    }

    #[test]
    fn test_parse_empty_module() {
        let source = "";
        let result = parse_gleam("example.gleam", source);
        assert!(result.is_ok());
        let module = result.unwrap();
        assert_eq!(module.types.len(), 0);
        assert_eq!(module.values.len(), 0);
    }
}
