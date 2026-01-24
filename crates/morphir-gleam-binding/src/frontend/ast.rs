//! Gleam AST types - Abstract Syntax Tree representation
//!
//! This module contains the AST types that represent parsed Gleam source code.

use serde::{Deserialize, Serialize};

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
