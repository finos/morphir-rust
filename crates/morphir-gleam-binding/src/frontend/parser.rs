#![allow(clippy::collapsible_str_replace)]
//! Gleam parser - converts Gleam source code to Morphir IR

use serde::{Deserialize, Serialize};

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

/// Parse error
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse Gleam source code into Morphir IR
///
/// This is a placeholder implementation that demonstrates the structure.
/// A real implementation would use a proper Gleam parser.
pub fn parse_gleam(path: &str, source: &str) -> Result<ModuleIR, ParseError> {
    // Extract module name from path
    let module_name = path
        .trim_end_matches(".gleam")
        .replace('/', "_")
        .replace('\\', "_");

    // Simple tokenization and parsing
    // This is a placeholder - a real implementation would use a proper parser
    let mut types = Vec::new();
    let mut values = Vec::new();

    // Parse simple function definitions
    for line in source.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        // Parse pub fn definitions
        if line.starts_with("pub fn ") {
            if let Some(name) = parse_function_name(line) {
                values.push(ValueDef {
                    name: name.to_string(),
                    type_annotation: None,
                    body: Expr::Literal {
                        value: Literal::String {
                            value: "placeholder".to_string(),
                        },
                    },
                });
            }
        }

        // Parse type definitions
        if line.starts_with("pub type ") {
            if let Some(name) = parse_type_name(line) {
                types.push(TypeDef {
                    name: name.to_string(),
                    params: vec![],
                    body: TypeExpr::Unit,
                });
            }
        }
    }

    Ok(ModuleIR {
        name: module_name,
        doc: None,
        types,
        values,
    })
}

/// Extract function name from a line like "pub fn hello() {"
fn parse_function_name(line: &str) -> Option<&str> {
    let line = line.strip_prefix("pub fn ")?;
    let end = line.find('(')?;
    Some(&line[..end])
}

/// Extract type name from a line like "pub type Person {"
fn parse_type_name(line: &str) -> Option<&str> {
    let line = line.strip_prefix("pub type ")?;
    let end = line
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(line.len());
    Some(&line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_module() {
        let source = r#"
pub fn hello() {
    "world"
}

pub type Person {
    Person(name: String, age: Int)
}
"#;

        let result = parse_gleam("example.gleam", source).unwrap();
        assert_eq!(result.name, "example");
        assert_eq!(result.values.len(), 1);
        assert_eq!(result.values[0].name, "hello");
        assert_eq!(result.types.len(), 1);
        assert_eq!(result.types[0].name, "Person");
    }
}
