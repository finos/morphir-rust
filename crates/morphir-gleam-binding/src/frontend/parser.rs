//! Gleam parser - converts Gleam source code to Morphir IR
//!
//! This implementation uses `logos` for lexing and `chumsky` for parsing,
//! following patterns from the official Gleam implementations (glexer and glance).

use chumsky::input::{IterInput, ValueInput};
use chumsky::prelude::*;
use chumsky::span::SimpleSpan;
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
#[logos(skip r"[ \t\r\n]+")]
#[logos(skip r"//[^\n]*")]
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
    #[regex(r"[a-z][a-zA-Z0-9_]*", priority = 2, callback = |lex| lex.slice().to_string())]
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

    /// Error token for unrecognized input
    Error,
}

/// Span type for tracking source positions
pub type Span = Range<usize>;

/// Token with span information
pub type SpannedToken = (Token, SimpleSpan);

/// Tokenize Gleam source code with spans for chumsky 0.12
pub fn tokenize(source: &str) -> Vec<SpannedToken> {
    Token::lexer(source)
        .spanned()
        .map(|(tok, span)| (tok.unwrap_or(Token::Error), span.into()))
        .collect()
}

// Error token variant needs to be added
impl Default for Token {
    fn default() -> Self {
        Token::Error
    }
}

// ============================================================================
// Parser (Grammar)
// ============================================================================

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

/// Convert chumsky Rich error to ParseError
fn to_parse_error(err: &Rich<'_, Token, SimpleSpan>, source: &str) -> ParseError {
    let span = err.span();
    let span_range = span.start..span.end;

    // Extract expected tokens
    let expected: Vec<String> = err
        .expected()
        .map(|e| format!("{:?}", e))
        .collect();

    let found = err.found().map(|t| format!("{:?}", t));

    // Extract source snippet for context
    let snippet = if span.start < source.len() && span.end <= source.len() {
        Some(source[span_range.clone()].to_string())
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
        message: format!("Parse error: {:?}", err.reason()),
        span: span_range,
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

    // Create end-of-input span
    let eoi = SimpleSpan::from(source.len()..source.len());

    // Create IterInput from tokens for parsing (handles (Token, Span) tuples)
    let input = IterInput::new(tokens.into_iter(), eoi);

    // Parse using chumsky 0.12 API
    let parser = module_parser();
    match parser.parse(input).into_result() {
        Ok(mut module) => {
            // Set module name from path
            module.name = extract_module_name(path);
            Ok(module)
        }
        Err(errors) => {
            // Return first error (could be enhanced to return multiple)
            if let Some(err) = errors.first() {
                Err(to_parse_error(err, source))
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
// Parser Combinators (Chumsky 0.12 API)
// ============================================================================

/// Statement enum
#[derive(Debug, Clone)]
enum Statement {
    TypeDef(TypeDef),
    ValueDef(ValueDef),
}

/// Main module parser
fn module_parser<'src, I>(
) -> impl Parser<'src, I, ModuleIR, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    // Parse module-level statements
    let stmt = statement_parser().then_ignore(just(Token::Semicolon).or_not());

    stmt.repeated()
        .collect::<Vec<_>>()
        .map(|stmts| {
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
fn statement_parser<'src, I>(
) -> impl Parser<'src, I, Statement, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    type_def_parser()
        .map(Statement::TypeDef)
        .or(value_def_parser().map(Statement::ValueDef))
}

/// Type definition parser
fn type_def_parser<'src, I>(
) -> impl Parser<'src, I, TypeDef, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    let access = just(Token::Pub)
        .to(Access::Public)
        .or_not()
        .map(|opt| opt.unwrap_or(Access::Private));

    let type_params = identifier_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .or_not()
        .map(|opt: Option<Vec<String>>| opt.unwrap_or_default());

    access
        .then_ignore(just(Token::Type))
        .then(type_identifier_parser())
        .then(type_params)
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
/// In Gleam, variants are listed consecutively without separators (whitespace is skipped)
fn custom_type_body_parser<'src, I>(
) -> impl Parser<'src, I, TypeExpr, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    variant_parser()
        .repeated()
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|variants| TypeExpr::CustomType { variants })
}

/// Variant parser
fn variant_parser<'src, I>(
) -> impl Parser<'src, I, Variant, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    let variant_fields = type_expr_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .or_not()
        .map(|opt: Option<Vec<TypeExpr>>| opt.unwrap_or_default());

    type_identifier_parser()
        .then(variant_fields)
        .map(|(name, fields)| Variant { name, fields })
}

/// Value definition parser
fn value_def_parser<'src, I>(
) -> impl Parser<'src, I, ValueDef, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    let access = just(Token::Pub)
        .to(Access::Public)
        .or_not()
        .map(|opt| opt.unwrap_or(Access::Private));

    let params = identifier_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<String>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .or_not()
        .map(|opt: Option<Vec<String>>| opt.unwrap_or_default());

    let type_ann = just(Token::Colon)
        .ignore_then(type_expr_parser())
        .or_not();

    access
        .then_ignore(just(Token::Fn))
        .then(identifier_parser())
        .then(params)
        .then(type_ann)
        .then_ignore(just(Token::LBrace))
        .then(expr_parser())
        .then_ignore(just(Token::RBrace))
        .map(|((((access, name), _params), type_annotation), body)| ValueDef {
            name,
            type_annotation,
            body,
            access,
        })
}

/// Expression parser
fn expr_parser<'src, I>(
) -> impl Parser<'src, I, Expr, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    recursive(|expr| {
        // Literals
        let literal = literal_parser().map(|lit| Expr::Literal { value: lit });

        // Variables
        let variable = identifier_parser().map(|name| Expr::Variable { name });

        // Constructors (uppercase identifiers)
        let constructor = type_identifier_parser().map(|name| Expr::Constructor { name });

        // Tuples: #(expr, expr, ...)
        let tuple = just(Token::Hash)
            .ignore_then(
                expr.clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| Expr::Tuple { elements });

        // Records: { field: expr, ... }
        let record_field = identifier_parser()
            .then_ignore(just(Token::Colon))
            .then(expr.clone());

        let record = record_field
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(|fields| Expr::Record { fields });

        // Lambda: fn(param) { expr }
        let lambda = just(Token::Fn)
            .ignore_then(
                identifier_parser().delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .then_ignore(just(Token::LBrace))
            .then(expr.clone())
            .then_ignore(just(Token::RBrace))
            .map(|(param, body)| Expr::Lambda {
                param,
                body: Box::new(body),
            });

        // Let binding: let name = expr { expr }
        let let_binding = just(Token::Let)
            .ignore_then(identifier_parser())
            .then_ignore(just(Token::Equals))
            .then(expr.clone())
            .then(
                expr.clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map(|((name, value), body)| Expr::Let {
                name,
                value: Box::new(value),
                body: Box::new(body),
            });

        // If expression: if expr { expr } else { expr }
        let if_expr = just(Token::If)
            .ignore_then(expr.clone())
            .then(
                expr.clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .then_ignore(just(Token::Else))
            .then(
                expr.clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map(|((condition, then_branch), else_branch)| Expr::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            });

        // Case expression: case expr { pattern -> expr, ... }
        let case_branch = pattern_parser()
            .then_ignore(just(Token::Arrow))
            .then(expr.clone())
            .map(|(pattern, body)| CaseBranch { pattern, body });

        let case_expr = just(Token::Case)
            .ignore_then(expr.clone())
            .then(
                case_branch
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map(|(subject, branches)| Expr::Case {
                subject: Box::new(subject),
                branches,
            });

        // Parenthesized expression
        let paren_expr = expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        // Primary expressions (atoms)
        let atom = literal
            .or(variable)
            .or(constructor)
            .or(tuple)
            .or(record)
            .or(paren_expr);

        // Field access: expr.field
        let field_access = just(Token::Dot).ignore_then(identifier_parser());

        // Function application: expr(expr)
        let application = expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        // Build up expressions with postfix operators
        let postfix = atom.foldl(
            field_access
                .map(|f| PostfixOp::Field(f))
                .or(application.map(|a| PostfixOp::Apply(a)))
                .repeated(),
            |lhs, op| match op {
                PostfixOp::Field(field) => Expr::Field {
                    record: Box::new(lhs),
                    field,
                },
                PostfixOp::Apply(argument) => Expr::Apply {
                    function: Box::new(lhs),
                    argument: Box::new(argument),
                },
            },
        );

        // All expression forms
        postfix.or(lambda).or(let_binding).or(if_expr).or(case_expr).boxed()
    })
}

/// Helper enum for postfix operators
#[derive(Clone)]
enum PostfixOp {
    Field(String),
    Apply(Expr),
}

/// Type expression parser
fn type_expr_parser<'src, I>(
) -> impl Parser<'src, I, TypeExpr, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    recursive(|type_expr| {
        // Type variable (lowercase)
        let type_var = identifier_parser().map(|name| TypeExpr::Variable { name });

        // Unit type: ()
        let unit = just(Token::LParen)
            .then(just(Token::RParen))
            .to(TypeExpr::Unit);

        // Tuple type: #(Type, Type, ...)
        let tuple_type = just(Token::Hash)
            .ignore_then(
                type_expr
                    .clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| TypeExpr::Tuple { elements });

        // Record type: { field: Type, ... }
        let record_field = identifier_parser()
            .then_ignore(just(Token::Colon))
            .then(type_expr.clone());

        let record_type = record_field
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(|fields| TypeExpr::Record { fields });

        // Reference type: TypeName or TypeName(Type, ...)
        let type_args = type_expr
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .or_not()
            .map(|opt: Option<Vec<TypeExpr>>| opt.unwrap_or_default());

        let ref_type = type_identifier_parser()
            .then(type_args)
            .map(|(name, args)| TypeExpr::Reference { name, args });

        // Parenthesized type
        let paren_type = type_expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        // Primary types
        let primary = unit
            .or(tuple_type)
            .or(record_type)
            .or(ref_type)
            .or(type_var)
            .or(paren_type);

        // Function types: Type -> Type (right-associative)
        primary.foldl(
            just(Token::Arrow)
                .ignore_then(type_expr)
                .repeated(),
            |from, to| TypeExpr::Function {
                from: Box::new(from),
                to: Box::new(to),
            },
        ).boxed()
    })
}

/// Pattern parser
fn pattern_parser<'src, I>(
) -> impl Parser<'src, I, Pattern, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    recursive(|pattern| {
        // Wildcard: _
        let wildcard = just(Token::Underscore).to(Pattern::Wildcard);

        // Variable pattern (lowercase)
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
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| Pattern::Tuple { elements });

        // Constructor pattern: ConstructorName or ConstructorName(pattern, ...)
        let constructor_args = pattern
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .or_not()
            .map(|opt: Option<Vec<Pattern>>| opt.unwrap_or_default());

        let constructor_pattern = type_identifier_parser()
            .then(constructor_args)
            .map(|(name, args)| Pattern::Constructor { name, args });

        // Parenthesized pattern
        let paren_pattern = pattern
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        wildcard
            .or(lit_pattern)
            .or(tuple_pattern)
            .or(constructor_pattern)
            .or(var_pattern)
            .or(paren_pattern)
            .boxed()
    })
}

/// Literal parser
fn literal_parser<'src, I>(
) -> impl Parser<'src, I, Literal, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    select! {
        Token::True => Literal::Bool { value: true },
        Token::False => Literal::Bool { value: false },
        Token::Int(i) => Literal::Int { value: i },
        Token::Float(f) => Literal::Float { value: f },
        Token::String(s) => Literal::String { value: s },
    }
    .labelled("literal")
}

/// Identifier parser (lowercase)
fn identifier_parser<'src, I>(
) -> impl Parser<'src, I, String, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    select! {
        Token::Ident(name) => name,
    }
    .labelled("identifier")
}

/// Type identifier parser (uppercase)
fn type_identifier_parser<'src, I>(
) -> impl Parser<'src, I, String, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    select! {
        Token::TypeIdent(name) => name,
    }
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
pub type Maybe {
    Just
    Nothing
}
"#;

        let result = parse_gleam("example.gleam", source);
        assert!(result.is_ok());
        let module = result.unwrap();
        assert_eq!(module.types.len(), 1);
        assert_eq!(module.types[0].name, "Maybe");
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
