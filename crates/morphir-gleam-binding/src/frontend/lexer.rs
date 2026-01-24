//! Gleam lexer - tokenizes Gleam source code
//!
//! This implementation uses `logos` for lexing, following patterns from
//! the official Gleam lexer (glexer).
//! Reference: https://github.com/gleam-lang/glexer

use chumsky::span::SimpleSpan;
use logos::Logos;
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
#[derive(Default)]
pub enum Token {
    // Comments (captured, not skipped) - ordered by priority for longest match
    #[regex(r"////[^\n]*", priority = 4, allow_greedy = true, callback = |lex| lex.slice()[4..].to_string())]
    CommentModule(String),
    #[regex(r"///[^\n]*", priority = 3, allow_greedy = true, callback = |lex| lex.slice()[3..].to_string())]
    CommentDoc(String),
    #[regex(r"//[^\n]*", priority = 2, allow_greedy = true, callback = |lex| lex.slice()[2..].to_string())]
    CommentNormal(String),

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
    // Additional keywords from glexer
    #[token("auto")]
    Auto,
    #[token("delegate")]
    Delegate,
    #[token("derive")]
    Derive,
    #[token("echo")]
    Echo,
    #[token("implement")]
    Implement,
    #[token("macro")]
    Macro,
    #[token("opaque")]
    Opaque,
    #[token("panic")]
    Panic,
    #[token("test")]
    Test,

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

    // Discard name pattern (e.g., _unused, _name)
    #[regex(r"_[a-zA-Z0-9_]+", priority = 3, callback = |lex| lex.slice().to_string())]
    DiscardName(String),

    // Operators
    #[token("->")]
    Arrow,
    #[token("<-")]
    LeftArrow,
    #[token("=")]
    Equals,
    #[token("|>")]
    PipeRight,
    #[token("|")]
    Pipe,
    #[token("_", priority = 1)]
    Underscore,
    #[token("::")]
    Cons,
    #[token("..")]
    Spread,
    // Float operators (higher priority to match before integer versions)
    #[token("+.")]
    PlusDot,
    #[token("-.")]
    MinusDot,
    #[token("*.")]
    StarDot,
    #[token("/.")]
    SlashDot,
    // Integer arithmetic operators
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
    // Float comparison operators (higher priority)
    #[token("<=.")]
    LtEqDot,
    #[token(">=.")]
    GtEqDot,
    #[token("<.")]
    LtDot,
    #[token(">.")]
    GtDot,
    // Integer comparison operators
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<=")]
    LtEq,
    #[token(">=")]
    GtEq,
    #[token("<>")]
    Concatenate,
    #[token("<<")]
    LeftShift,
    #[token(">>")]
    RightShift,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("!")]
    Not,
    #[token("?")]
    Question,
    #[token("@")]
    At,

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
    #[default]
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
