//! Gleam AST types - Abstract Syntax Tree representation
//!
//! This module contains the AST types that represent parsed Gleam source code.
//! Based on glance (official Gleam parser) type definitions.
//! Reference: https://github.com/lpil/glance

use serde::{Deserialize, Serialize};

// ============================================================================
// Core Types
// ============================================================================

/// Source location span (byte offsets)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl From<std::ops::Range<usize>> for Span {
    fn from(range: std::ops::Range<usize>) -> Self {
        Span {
            start: range.start,
            end: range.end,
        }
    }
}

// ============================================================================
// Binary Operators (matching glance)
// ============================================================================

/// Binary operator types (matching glance BinaryOperator)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinaryOperator {
    // Logical
    And,
    Or,
    // Comparison (equality)
    Eq,
    NotEq,
    // Comparison (integer)
    LtInt,
    LtEqInt,
    GtInt,
    GtEqInt,
    // Comparison (float)
    LtFloat,
    LtEqFloat,
    GtFloat,
    GtEqFloat,
    // Arithmetic (integer)
    AddInt,
    SubInt,
    MultInt,
    DivInt,
    RemainderInt,
    // Arithmetic (float)
    AddFloat,
    SubFloat,
    MultFloat,
    DivFloat,
    // Other
    Pipe,
    Concatenate,
}

// ============================================================================
// Field Type (for labelled arguments)
// ============================================================================

/// Field in function calls or record construction (matching glance Field)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Field<T> {
    /// Labelled field: `label: value`
    Labelled { label: String, item: T },
    /// Shorthand field: `label` (same as `label: label`)
    Shorthand { name: String },
    /// Unlabelled field: just the value
    Unlabelled { item: T },
}

// ============================================================================
// Statement Type (for function bodies)
// ============================================================================

/// Statement in a function body (matching glance Statement)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Statement {
    /// Use expression: `use pattern <- function`
    Use {
        patterns: Vec<Pattern>,
        function: Box<Expr>,
    },
    /// Let assignment: `let pattern = value`
    Assignment {
        pattern: Pattern,
        annotation: Option<TypeExpr>,
        value: Box<Expr>,
    },
    /// Expression statement
    Expression(Expr),
}

// ============================================================================
// BitString Segment (for bit string expressions/patterns)
// ============================================================================

/// Bit string segment option
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BitStringOption {
    Bytes,
    Int,
    Float,
    Bits,
    Utf8,
    Utf16,
    Utf32,
    Signed,
    Unsigned,
    Big,
    Little,
    Native,
    Size(Box<Expr>),
    Unit(u64),
}

/// Bit string segment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitStringSegment<T> {
    pub value: T,
    pub options: Vec<BitStringOption>,
}

// ============================================================================
// AST Types
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

/// Type expression (matching glance Type)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TypeExpr {
    /// Type variable (e.g., `a`)
    Variable { name: String },
    /// Unit type (Nil)
    Unit,
    /// Function type (e.g., `fn(Int) -> String`)
    Function {
        parameters: Vec<TypeExpr>,
        return_type: Box<TypeExpr>,
    },
    /// Record type (e.g., `{ name: String, age: Int }`)
    Record { fields: Vec<(String, TypeExpr)> },
    /// Tuple type (e.g., `#(Int, String)`)
    Tuple { elements: Vec<TypeExpr> },
    /// Reference to named type with optional module (e.g., `gleam/option.Option(Int)`)
    Named {
        module: Option<String>,
        name: String,
        parameters: Vec<TypeExpr>,
    },
    /// Custom type definition (variants)
    CustomType { variants: Vec<Variant> },
    /// Type hole (_)
    Hole { name: String },
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

/// Expression (matching glance Expression)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Expr {
    /// Literal value
    Literal { value: Literal },
    /// Variable reference
    Variable { name: String },
    /// Function application with labelled arguments
    Apply {
        function: Box<Expr>,
        arguments: Vec<Field<Expr>>,
    },
    /// Lambda expression
    Lambda {
        params: Vec<String>,
        body: Box<Expr>,
    },
    /// Let binding (legacy - use Statement::Assignment in blocks)
    Let {
        name: String,
        value: Box<Expr>,
        body: Box<Expr>,
    },
    /// If expression (Gleam doesn't have traditional if, uses case on bool)
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// Record construction
    Record { fields: Vec<(String, Expr)> },
    /// Record field access
    FieldAccess {
        container: Box<Expr>,
        label: String,
    },
    /// Tuple construction
    Tuple { elements: Vec<Expr> },
    /// Tuple index access (e.g., `tuple.0`)
    TupleIndex { tuple: Box<Expr>, index: u64 },
    /// Pattern match / case expression
    Case {
        subjects: Vec<Expr>,
        clauses: Vec<CaseBranch>,
    },
    /// Constructor reference
    Constructor {
        module: Option<String>,
        name: String,
    },
    /// Binary operator expression
    BinaryOp {
        op: BinaryOperator,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Integer negation
    NegateInt { value: Box<Expr> },
    /// Boolean negation
    NegateBool { value: Box<Expr> },
    /// List literal with optional tail
    List {
        elements: Vec<Expr>,
        tail: Option<Box<Expr>>,
    },
    /// Block expression (sequence of statements)
    Block { statements: Vec<Statement> },
    /// Panic expression
    Panic { message: Option<Box<Expr>> },
    /// Todo expression
    Todo { message: Option<Box<Expr>> },
    /// Echo expression (for debugging)
    Echo {
        expression: Box<Expr>,
        body: Option<Box<Expr>>,
    },
    /// Bit string literal
    BitString {
        segments: Vec<BitStringSegment<Expr>>,
    },
    /// Function capture (partial application with _)
    FnCapture {
        function: Box<Expr>,
        arguments_before: Vec<Field<Expr>>,
        arguments_after: Vec<Field<Expr>>,
    },
    /// Record update expression
    RecordUpdate {
        module: Option<String>,
        constructor: String,
        record: Box<Expr>,
        fields: Vec<(String, Expr)>,
    },
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

/// Pattern for matching (matching glance Pattern)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Pattern {
    /// Wildcard pattern (_)
    Wildcard,
    /// Variable binding
    Variable { name: String },
    /// Discard pattern with name (_name)
    Discard { name: String },
    /// Literal pattern
    Literal { value: Literal },
    /// Constructor pattern with optional module qualification
    Constructor {
        module: Option<String>,
        name: String,
        arguments: Vec<Field<Pattern>>,
        with_spread: bool,
    },
    /// Tuple pattern
    Tuple { elements: Vec<Pattern> },
    /// List pattern with optional tail
    List {
        elements: Vec<Pattern>,
        tail: Option<Box<Pattern>>,
    },
    /// Pattern assignment (pattern as name)
    Assignment {
        pattern: Box<Pattern>,
        name: String,
    },
    /// String concatenation pattern (prefix matching)
    Concatenate {
        prefix: String,
        suffix_assignment: Option<String>,
    },
    /// Bit string pattern
    BitString {
        segments: Vec<BitStringSegment<Pattern>>,
    },
}
