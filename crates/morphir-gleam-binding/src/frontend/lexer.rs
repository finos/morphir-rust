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
#[logos(skip r"//[^\n]*")]
#[derive(Default)]
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
