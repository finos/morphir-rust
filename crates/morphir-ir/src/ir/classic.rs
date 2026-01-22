//! Morphir IR data structures for Classic/Legacy format (V1/V2/V3).

use serde::{Deserialize, Serialize};
use crate::naming::Path;

/// Top-level Distribution wrapper with format version
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    pub format_version: u32,
    pub distribution: DistributionBody,
}

/// Distribution body - supports both "library" (V1) and "Library" (V2+) tags
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DistributionBody {
    Library(LibraryTag, Path, Vec<serde_json::Value>, Package),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LibraryTag {
    #[serde(alias = "library")]
    Library
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    #[serde(skip)]
    pub name: String,
    pub modules: Vec<Module>,
}

/// Module supports both V1 object format and V2+ tuple format
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Module {
    pub name: Path,
    pub detail: ModuleDetail,
}

impl<'de> Deserialize<'de> for Module {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Def { Tuple(Path, ModuleDetail), Obj { name: Path, def: ModuleDetail } }
        match Def::deserialize(deserializer)? {
            Def::Tuple(n, d) => Ok(Module { name: n, detail: d }),
            Def::Obj { name, def } => Ok(Module { name, detail: def }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleDetail { pub access: String, pub value: ModuleValue }

impl<'de> Deserialize<'de> for ModuleDetail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Def { Obj { access: String, value: ModuleValue }, Tuple(String, ModuleValue) }
        match Def::deserialize(deserializer)? {
            Def::Obj { access, value } => Ok(ModuleDetail { access, value }),
            Def::Tuple(a, v) => Ok(ModuleDetail { access: a, value: v }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleValue {
    #[serde(default)]
    pub types: Vec<serde_json::Value>,
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDefinition { pub name: String, pub typ: TypeExpression }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueDefinition { pub name: String, pub typ: TypeExpression, pub body: Expression }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TypeExpression {
    Unit,
    Variable { name: String },
    Reference { name: String, parameters: Vec<TypeExpression> },
    Function { parameter: Box<TypeExpression>, return_type: Box<TypeExpression> },
    Record { fields: Vec<Field> },
    Tuple { elements: Vec<TypeExpression> },
}

/// Value (expression) in Morphir IR.  V1: snake_case, V2/V3: TitleCase
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expression {
    Literal(Literal),
    Variable { name: String },
    Apply { function: Box<Expression>, argument: Box<Expression> },
    Lambda { parameter: String, body: Box<Expression>, in_expr: Box<Expression> },
    Let { bindings: Vec<Binding>, in_expr: Box<Expression> },
    IfThenElse { condition: Box<Expression>, then_expr: Box<Expression>, else_expr: Box<Expression> },
    PatternMatch { input: Box<Expression>, cases: Vec<PatternCase> },
    Record { fields: Vec<RecordField> },
    FieldAccess { record: Box<Expression>, field: String },
    Tuple { elements: Vec<Expression> },
    Unit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Literal { Bool(bool), Int(i64), Float(f64), String(String), Char(char), WholeNumber(i64) }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field { pub name: String, pub typ: TypeExpression }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding { pub name: String, pub expr: Expression }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternCase { pub pattern: Pattern, pub expr: Expression }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Pattern { Wildcard, Variable { name: String }, Literal { value: Literal }, Constructor { name: String, arguments: Vec<Pattern> }, Tuple { elements: Vec<Pattern> }, Record { fields: Vec<String> }, Unit }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordField { pub name: String, pub expr: Expression }

impl Package {
    pub fn new(name: String) -> Self { Self { name, modules: Vec::new() } }
    pub fn with_module(mut self, m: Module) -> Self { self.modules.push(m); self }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_v1_module() {
        let json = r#"{"name": [["rentals"]], "def": ["public", {"types":[], "values":[]}]}"#;
        let module: Result<Module, _> = serde_json::from_str(json);
        assert!(module.is_ok(), "Parse error: {:?}", module.err());
    }

    #[test]
    fn test_load_v1_distribution() {
        let json = r#"{"formatVersion": 1, "distribution": ["library", [["morphir"]], [], {"modules": [{"name": [["rentals"]], "def": ["public", {"types":[], "values":[]}]}]}]}"#;
        let dist: Result<Distribution, _> = serde_json::from_str(json);
        assert!(dist.is_ok(), "Parse error: {:?}", dist.err());
    }
}
