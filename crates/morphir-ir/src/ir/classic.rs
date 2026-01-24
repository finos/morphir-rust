//! Morphir IR data structures for Classic/Legacy format (V1/V2/V3).

use crate::naming::{Name, Path};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Classic Name type that serializes as array format.
///
/// In Classic format (V1/V2/V3), names are represented as arrays of word strings
/// like `["my", "type", "name"]` rather than the V4 kebab-case string `"my-type-name"`.
///
/// Use this when working with Classic IR serialization. For all other purposes,
/// use the standard `Name` type which serializes in V4 canonical format.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClassicName(pub Name);

impl ClassicName {
    /// Create a new ClassicName from a Name
    pub fn new(name: Name) -> Self {
        Self(name)
    }

    /// Get the inner Name
    pub fn inner(&self) -> &Name {
        &self.0
    }

    /// Convert to the standard Name type
    pub fn into_inner(self) -> Name {
        self.0
    }
}

impl std::fmt::Display for ClassicName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Name> for ClassicName {
    fn from(name: Name) -> Self {
        Self(name)
    }
}

impl From<ClassicName> for Name {
    fn from(classic: ClassicName) -> Self {
        classic.0
    }
}

impl From<&str> for ClassicName {
    fn from(s: &str) -> Self {
        Self(Name::from(s))
    }
}

impl Serialize for ClassicName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Classic format: serialize as array of words
        self.0.words.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ClassicName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        // Accept both array format (Classic) and string format (V4) for flexibility
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            // V4 canonical string format: "testModule" or "my-function"
            serde_json::Value::String(s) => Ok(ClassicName(Name::from(&s))),
            // Classic array format: ["test", "module"]
            serde_json::Value::Array(arr) => {
                let words: Result<Vec<String>, _> = arr
                    .into_iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => Ok(s),
                        _ => Err(de::Error::custom("expected string in ClassicName array")),
                    })
                    .collect();
                Ok(ClassicName(Name { words: words? }))
            }
            _ => Err(de::Error::custom(
                "expected string or array for ClassicName",
            )),
        }
    }
}

impl JsonSchema for ClassicName {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ClassicName".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::ClassicName").into()
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Classic format is an array of strings
        schemars::json_schema!({
            "type": "array",
            "items": { "type": "string" }
        })
    }
}

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
    Library,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Package {
    #[serde(skip)]
    pub name: String,
    pub modules: Vec<Module>,
}

/// Module - V1: {"name":..., "def":...}, V2+: [[path], {access, value}]
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: Path,
    pub detail: ModuleDetail,
}

impl Serialize for Module {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as tuple [path, detail] for V2+ format
        use serde::ser::SerializeTuple;
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&self.name)?;
        tuple.serialize_element(&self.detail)?;
        tuple.end()
    }
}

impl<'de> Deserialize<'de> for Module {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Def {
            Tuple(Path, ModuleDetail),
            Obj { name: Path, def: ModuleDetail },
        }
        match Def::deserialize(deserializer)? {
            Def::Tuple(n, d) => Ok(Module { name: n, detail: d }),
            Def::Obj { name, def } => Ok(Module { name, detail: def }),
        }
    }
}

/// ModuleDetail - V1: ["access", {...}], V2+: {access, value}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleDetail {
    pub access: String,
    pub value: ModuleValue,
}

impl<'de> Deserialize<'de> for ModuleDetail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Def {
            Obj { access: String, value: ModuleValue },
            Tuple(String, ModuleValue),
        }
        match Def::deserialize(deserializer)? {
            Def::Obj { access, value } => Ok(ModuleDetail { access, value }),
            Def::Tuple(a, v) => Ok(ModuleDetail {
                access: a,
                value: v,
            }),
        }
    }
}

/// Module content (flexible serde_json::Value for types/values)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleValue {
    #[serde(default)]
    pub types: Vec<serde_json::Value>,
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
    #[serde(default)]
    pub doc: Option<String>,
}

// Type/Value definition placeholders for visitor.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDefinition {
    pub name: String,
    pub typ: TypeExpression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueDefinition {
    pub name: String,
    pub typ: TypeExpression,
    pub body: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TypeExpression {
    Unit,
    Variable {
        name: String,
    },
    Reference {
        name: String,
        parameters: Vec<TypeExpression>,
    },
    Function {
        parameter: Box<TypeExpression>,
        return_type: Box<TypeExpression>,
    },
    Record {
        fields: Vec<Field>,
    },
    Tuple {
        elements: Vec<TypeExpression>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expression {
    Literal(Literal),
    Variable {
        name: String,
    },
    Apply {
        function: Box<Expression>,
        argument: Box<Expression>,
    },
    Lambda {
        parameter: String,
        body: Box<Expression>,
        in_expr: Box<Expression>,
    },
    Let {
        bindings: Vec<Binding>,
        in_expr: Box<Expression>,
    },
    IfThenElse {
        condition: Box<Expression>,
        then_expr: Box<Expression>,
        else_expr: Box<Expression>,
    },
    PatternMatch {
        input: Box<Expression>,
        cases: Vec<PatternCase>,
    },
    Record {
        fields: Vec<RecordField>,
    },
    FieldAccess {
        record: Box<Expression>,
        field: String,
    },
    Tuple {
        elements: Vec<Expression>,
    },
    Unit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Literal {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Char(char),
    WholeNumber(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub typ: TypeExpression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub name: String,
    pub expr: Expression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternCase {
    pub pattern: Pattern,
    pub expr: Expression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Pattern {
    Wildcard,
    Variable {
        name: String,
    },
    Literal {
        value: Literal,
    },
    Constructor {
        name: String,
        arguments: Vec<Pattern>,
    },
    Tuple {
        elements: Vec<Pattern>,
    },
    Record {
        fields: Vec<String>,
    },
    Unit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordField {
    pub name: String,
    pub expr: Expression,
}

impl Package {
    pub fn new(name: String) -> Self {
        Self {
            name,
            modules: Vec::new(),
        }
    }
    pub fn with_module(mut self, m: Module) -> Self {
        self.modules.push(m);
        self
    }
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

    #[test]
    #[ignore]
    fn test_load_real_v1() {
        let content = std::fs::read_to_string("../morphir-tests/tests/features/real_v1.json")
            .expect("Failed to read real_v1.json");
        let result: Result<Distribution, _> = serde_json::from_str(&content);
        if let Err(e) = &result {
            panic!(
                "Parse error at line {} column {}: {}",
                e.line(),
                e.column(),
                e
            );
        }
        let dist = result.unwrap();
        // Fixed irrefutable pattern
        let DistributionBody::Library(_, _, _, pkg) = &dist.distribution;
        assert!(!pkg.modules.is_empty(), "Should have at least one module");
    }
}
