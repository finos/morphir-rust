//! Morphir IR V4
//!
//! This module defines the structure for Morphir IR Version 4.
//! It supports the Document Tree structure and Canonical Strings.
//!
//! V4 uses object wrapper format for enums and keyed objects (IndexMap) for
//! dictionaries rather than arrays of tuples.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::attributes::ValueAttributes;
use super::type_expr::Type;
use super::value_expr::Value;

// Re-export naming types - Name now serializes as V4 canonical format (kebab-case string)
pub use crate::naming::ModuleName;
pub use crate::naming::Name;
pub use crate::naming::PackageName;
pub use crate::naming::Path;

/// Top-level IR file structure
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IRFile {
    pub format_version: FormatVersion,
    pub distribution: Distribution,
}

/// Format version - accepts both string "4.0.0" and integer 4
#[derive(Debug, Clone, PartialEq, JsonSchema)]
pub enum FormatVersion {
    String(String),
    Integer(u32),
}

impl Serialize for FormatVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            FormatVersion::String(s) => serializer.serialize_str(s),
            FormatVersion::Integer(n) => serializer.serialize_u32(*n),
        }
    }
}

impl<'de> Deserialize<'de> for FormatVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => Ok(FormatVersion::String(s)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_u64() {
                    Ok(FormatVersion::Integer(i as u32))
                } else {
                    Err(de::Error::custom(
                        "format version must be a positive integer",
                    ))
                }
            }
            _ => Err(de::Error::custom(
                "format version must be a string or integer",
            )),
        }
    }
}

impl Default for FormatVersion {
    fn default() -> Self {
        FormatVersion::String("4.0.0".to_string())
    }
}

/// Distribution enum - serializes as wrapper object format
/// E.g., `{ "Library": { ... } }` or `{ "Specs": { ... } }`
#[derive(Debug, Clone, PartialEq)]
pub enum Distribution {
    Library(LibraryContent),
    Specs(SpecsContent),
    Application(ApplicationContent),
}

impl Serialize for Distribution {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Distribution::Library(content) => {
                map.serialize_entry("Library", content)?;
            }
            Distribution::Specs(content) => {
                map.serialize_entry("Specs", content)?;
            }
            Distribution::Application(content) => {
                map.serialize_entry("Application", content)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Distribution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(DistributionVisitor)
    }
}

struct DistributionVisitor;

impl<'de> Visitor<'de> for DistributionVisitor {
    type Value = Distribution;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a distribution object wrapper like { \"Library\": { ... } }")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Distribution, M::Error>
    where
        M: MapAccess<'de>,
    {
        let (key, value): (String, serde_json::Value) = map
            .next_entry()?
            .ok_or_else(|| de::Error::custom("expected distribution wrapper object"))?;

        match key.as_str() {
            "Library" => {
                let content: LibraryContent =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Distribution::Library(content))
            }
            "Specs" => {
                let content: SpecsContent =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Distribution::Specs(content))
            }
            "Application" => {
                let content: ApplicationContent =
                    serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(Distribution::Application(content))
            }
            _ => Err(de::Error::unknown_variant(
                &key,
                &["Library", "Specs", "Application"],
            )),
        }
    }
}

/// Library distribution content
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryContent {
    pub package_name: PackageName,
    pub dependencies: Dependencies,
    pub def: PackageDefinition,
}

/// Specs distribution content (public interfaces only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecsContent {
    pub package_name: PackageName,
    pub dependencies: Dependencies,
    pub spec: PackageSpecification,
}

/// Application distribution content
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationContent {
    pub package_name: PackageName,
    pub dependencies: Dependencies,
    pub def: PackageDefinition,
    pub entry_points: EntryPoints,
}

/// Dependencies as keyed object: `{ "morphir/sdk": { modules: ... } }`
pub type Dependencies = IndexMap<String, PackageSpecification>;

/// Package specification (for dependencies)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSpecification {
    pub modules: IndexMap<String, ModuleSpecification>,
}

/// Module specification (public API only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSpecification {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    pub types: IndexMap<String, TypeSpecification>,
    pub values: IndexMap<String, ValueSpecification>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Annotation on a specification (V4 feature)
///
/// Annotations provide structured metadata for types and values.
/// Can be compact string format "package:module#name" or canonical object format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Annotation {
    /// Compact string format: "package:module#name" or "package:module#name:value"
    Compact(String),
    /// Canonical object format with name and optional arguments
    Canonical {
        name: String, // FQName as canonical string
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        arguments: Vec<AnnotationArgument>,
    },
}

/// Argument to an annotation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AnnotationArgument {
    /// Positional argument (just a value)
    Positional(serde_json::Value), // TODO: Use Value when serde is complete
    /// Named argument
    Named {
        name: Name,
        value: serde_json::Value, // TODO: Use Value when serde is complete
    },
}

/// Type specification (public API view of a type)
// The variant names include "Specification" suffix as per the Morphir specification
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeSpecification {
    /// Type alias specification
    TypeAliasSpecification {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        annotations: Vec<Annotation>,
        type_params: Vec<Name>,
        #[serde(rename = "typeExp")]
        type_expr: Type,
    },
    /// Opaque type (constructors hidden)
    OpaqueTypeSpecification {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        annotations: Vec<Annotation>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        type_params: Vec<Name>,
    },
    /// Custom type with public constructors
    CustomTypeSpecification {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        annotations: Vec<Annotation>,
        type_params: Vec<Name>,
        constructors: Vec<ConstructorSpecification>,
    },
    /// Derived type (opaque with conversion functions)
    DerivedTypeSpecification {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        annotations: Vec<Annotation>,
        type_params: Vec<Name>,
        base_type: serde_json::Value, // TODO: Use TypeExpr when serde is complete
        from_base_type: String,       // FQName as canonical string
        to_base_type: String,         // FQName as canonical string
    },
}

/// Constructor specification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorSpecification {
    pub name: Name,
    pub args: Vec<ConstructorArgSpec>,
}

/// Constructor argument specification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorArgSpec {
    pub name: Name,
    #[serde(rename = "type")]
    pub arg_type: Type,
}

/// Value specification (just the signature)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueSpecification {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub inputs: IndexMap<String, Type>,
    pub output: Type,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Package definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDefinition {
    pub modules: IndexMap<String, AccessControlledModuleDefinition>,
}

/// Access-controlled module definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessControlledModuleDefinition {
    pub access: Access,
    pub value: ModuleDefinition,
}

/// Module definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDefinition {
    pub types: IndexMap<String, AccessControlledTypeDefinition>,
    pub values: IndexMap<String, AccessControlledValueDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Access-controlled type definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessControlledTypeDefinition {
    pub access: Access,
    #[serde(flatten)]
    pub value: TypeDefinition,
}

/// Type definition - uses wrapper object format
// The variant names include "Definition" suffix as per the Morphir specification
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, JsonSchema)]
pub enum TypeDefinition {
    TypeAliasDefinition {
        type_params: Vec<Name>,
        type_expr: Type,
    },
    CustomTypeDefinition {
        type_params: Vec<Name>,
        constructors: AccessControlledConstructors,
    },
    /// Incomplete type definition (V4 only)
    IncompleteTypeDefinition {
        type_params: Vec<Name>,
        incompleteness: Incompleteness,
        #[serde(skip_serializing_if = "Option::is_none")]
        partial_type_exp: Option<serde_json::Value>,
    },
}

impl Serialize for TypeDefinition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            TypeDefinition::TypeAliasDefinition {
                type_params,
                type_expr,
            } => {
                map.serialize_entry(
                    "TypeAliasDefinition",
                    &TypeAliasDefContent {
                        type_params: type_params.clone(),
                        type_exp: type_expr.clone(),
                    },
                )?;
            }
            TypeDefinition::CustomTypeDefinition {
                type_params,
                constructors,
            } => {
                map.serialize_entry(
                    "CustomTypeDefinition",
                    &CustomTypeDefContent {
                        type_params: type_params.clone(),
                        constructors: constructors.clone(),
                    },
                )?;
            }
            TypeDefinition::IncompleteTypeDefinition {
                type_params,
                incompleteness,
                partial_type_exp,
            } => {
                map.serialize_entry(
                    "IncompleteTypeDefinition",
                    &IncompleteTypeDefContent {
                        type_params: type_params.clone(),
                        incompleteness: incompleteness.clone(),
                        partial_type_exp: partial_type_exp.clone(),
                    },
                )?;
            }
        }
        map.end()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TypeAliasDefContent {
    type_params: Vec<Name>,
    type_exp: Type,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomTypeDefContent {
    type_params: Vec<Name>,
    constructors: AccessControlledConstructors,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteTypeDefContent {
    type_params: Vec<Name>,
    incompleteness: Incompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    partial_type_exp: Option<serde_json::Value>,
}

impl<'de> Deserialize<'de> for TypeDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::Object(map) = &value {
            if let Some(content) = map.get("TypeAliasDefinition") {
                let parsed: TypeAliasDefContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(TypeDefinition::TypeAliasDefinition {
                    type_params: parsed.type_params,
                    type_expr: parsed.type_exp,
                });
            }
            if let Some(content) = map.get("CustomTypeDefinition") {
                let parsed: CustomTypeDefContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(TypeDefinition::CustomTypeDefinition {
                    type_params: parsed.type_params,
                    constructors: parsed.constructors,
                });
            }
            if let Some(content) = map.get("IncompleteTypeDefinition") {
                let parsed: IncompleteTypeDefContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(TypeDefinition::IncompleteTypeDefinition {
                    type_params: parsed.type_params,
                    incompleteness: parsed.incompleteness,
                    partial_type_exp: parsed.partial_type_exp,
                });
            }
        }
        Err(de::Error::custom(
            "expected TypeAliasDefinition, CustomTypeDefinition, or IncompleteTypeDefinition wrapper",
        ))
    }
}

impl Serialize for Incompleteness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Incompleteness::Hole { reason } => {
                map.serialize_entry("Hole", &serde_json::json!({ "reason": reason }))?;
            }
            Incompleteness::Draft => {
                map.serialize_entry("Draft", &serde_json::json!({}))?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Incompleteness {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match &value {
            serde_json::Value::Object(map) => {
                if let Some((key, content)) = map.iter().next() {
                    match key.as_str() {
                        "Hole" => {
                            let reason_val = content
                                .get("reason")
                                .ok_or_else(|| de::Error::missing_field("reason"))?;
                            let reason: HoleReason = serde_json::from_value(reason_val.clone())
                                .map_err(de::Error::custom)?;
                            Ok(Incompleteness::Hole { reason })
                        }
                        "Draft" => Ok(Incompleteness::Draft),
                        _ => Err(de::Error::unknown_variant(key, &["Hole", "Draft"])),
                    }
                } else {
                    Err(de::Error::custom("empty object for Incompleteness"))
                }
            }
            _ => Err(de::Error::custom("expected object for Incompleteness")),
        }
    }
}

/// Access-controlled constructors
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessControlledConstructors {
    pub access: Access,
    pub value: Vec<ConstructorDefinition>,
}

/// Constructor definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorDefinition {
    pub name: Name,
    pub args: Vec<ConstructorArg>,
}

/// Constructor argument
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstructorArg {
    pub name: Name,
    #[serde(rename = "type")]
    pub arg_type: Type,
}

/// Access-controlled value definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessControlledValueDefinition {
    pub access: Access,
    #[serde(flatten)]
    pub value: ValueDefinition,
}

/// Value definition - uses wrapper object format for body
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueDefinition {
    pub input_types: IndexMap<String, InputTypeEntry>,
    pub output_type: Type,
    pub body: ValueBody,
}

/// Input type entry
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputTypeEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_attributes: Option<ValueAttributes>,
    #[serde(rename = "type")]
    pub input_type: Type,
}

/// Value body - uses wrapper object format
// The variant names include "Body" suffix as per the Morphir specification
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq)]
pub enum ValueBody {
    ExpressionBody { body: Value },
    NativeBody {
        hint: NativeHint,
        description: Option<String>,
    },
    ExternalBody {
        external_name: String,
        target_platform: String,
    },
    IncompleteBody {
        reason: HoleReason,
    },
}

impl Serialize for ValueBody {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            ValueBody::ExpressionBody { body } => {
                map.serialize_entry("ExpressionBody", &ExpressionBodySerContent { body })?;
            }
            ValueBody::NativeBody { hint, description } => {
                map.serialize_entry(
                    "NativeBody",
                    &NativeBodySerContent {
                        hint: hint.clone(),
                        description: description.clone(),
                    },
                )?;
            }
            ValueBody::ExternalBody {
                external_name,
                target_platform,
            } => {
                map.serialize_entry(
                    "ExternalBody",
                    &ExternalBodySerContent {
                        external_name: external_name.clone(),
                        target_platform: target_platform.clone(),
                    },
                )?;
            }
            ValueBody::IncompleteBody { reason } => {
                map.serialize_entry("IncompleteBody", &IncompleteBodySerContent { reason })?;
            }
        }
        map.end()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpressionBodySerContent<'a> {
    body: &'a Value,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeBodySerContent {
    hint: NativeHint,
    description: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExternalBodySerContent {
    external_name: String,
    target_platform: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteBodySerContent<'a> {
    reason: &'a HoleReason,
}

impl<'de> Deserialize<'de> for ValueBody {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::Object(map) = &value {
            if let Some(content) = map.get("ExpressionBody") {
                let body_json = content
                    .get("body")
                    .ok_or_else(|| de::Error::missing_field("body"))?;
                let body: Value =
                    serde_json::from_value(body_json.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::ExpressionBody { body });
            }
            if let Some(content) = map.get("NativeBody") {
                let parsed: NativeBodySerContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::NativeBody {
                    hint: parsed.hint,
                    description: parsed.description,
                });
            }
            if let Some(content) = map.get("ExternalBody") {
                let parsed: ExternalBodySerContent =
                    serde_json::from_value(content.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::ExternalBody {
                    external_name: parsed.external_name,
                    target_platform: parsed.target_platform,
                });
            }
            if let Some(content) = map.get("IncompleteBody") {
                let reason_val = content
                    .get("reason")
                    .ok_or_else(|| de::Error::missing_field("reason"))?;
                let reason: HoleReason =
                    serde_json::from_value(reason_val.clone()).map_err(de::Error::custom)?;
                return Ok(ValueBody::IncompleteBody { reason });
            }
        }
        Err(de::Error::custom(
            "expected ExpressionBody, NativeBody, ExternalBody, or IncompleteBody wrapper",
        ))
    }
}

/// Native hint (V4 uses wrapper object format)
///
/// Categorization hint for native operations used by code generators.
#[derive(Debug, Clone, PartialEq, JsonSchema)]
pub enum NativeHint {
    /// Basic arithmetic/logic operation
    Arithmetic,
    /// Comparison operation
    Comparison,
    /// String operation
    StringOp,
    /// Collection operation
    CollectionOp,
    /// Platform-specific operation
    PlatformSpecific { platform: String },
}

/// Incompleteness reason for type or value definitions (V4 feature)
///
/// Used to indicate why a definition couldn't be fully resolved.
#[derive(Debug, Clone, PartialEq, JsonSchema)]
pub enum Incompleteness {
    /// Hole due to unresolved reference or error
    Hole { reason: HoleReason },
    /// Author-marked work-in-progress
    Draft,
}

impl Serialize for NativeHint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            NativeHint::Arithmetic => map.serialize_entry("Arithmetic", &serde_json::json!({}))?,
            NativeHint::Comparison => map.serialize_entry("Comparison", &serde_json::json!({}))?,
            NativeHint::StringOp => map.serialize_entry("StringOp", &serde_json::json!({}))?,
            NativeHint::CollectionOp => {
                map.serialize_entry("CollectionOp", &serde_json::json!({}))?
            }
            NativeHint::PlatformSpecific { platform } => map.serialize_entry(
                "PlatformSpecific",
                &serde_json::json!({ "platform": platform }),
            )?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for NativeHint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match &value {
            serde_json::Value::Object(map) => {
                if let Some((key, content)) = map.iter().next() {
                    match key.as_str() {
                        "Arithmetic" => Ok(NativeHint::Arithmetic),
                        "Comparison" => Ok(NativeHint::Comparison),
                        "StringOp" => Ok(NativeHint::StringOp),
                        "CollectionOp" => Ok(NativeHint::CollectionOp),
                        "PlatformSpecific" => {
                            let platform = content
                                .get("platform")
                                .and_then(|p| p.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            Ok(NativeHint::PlatformSpecific { platform })
                        }
                        _ => Err(de::Error::unknown_variant(
                            key,
                            &[
                                "Arithmetic",
                                "Comparison",
                                "StringOp",
                                "CollectionOp",
                                "PlatformSpecific",
                            ],
                        )),
                    }
                } else {
                    Err(de::Error::custom("empty object for NativeHint"))
                }
            }
            // Also accept string format for backward compatibility (without platform)
            serde_json::Value::String(s) => match s.as_str() {
                "Arithmetic" => Ok(NativeHint::Arithmetic),
                "Comparison" => Ok(NativeHint::Comparison),
                "StringOp" => Ok(NativeHint::StringOp),
                "CollectionOp" => Ok(NativeHint::CollectionOp),
                "PlatformSpecific" => Ok(NativeHint::PlatformSpecific {
                    platform: "unknown".to_string(),
                }),
                _ => Err(de::Error::unknown_variant(
                    s,
                    &[
                        "Arithmetic",
                        "Comparison",
                        "StringOp",
                        "CollectionOp",
                        "PlatformSpecific",
                    ],
                )),
            },
            _ => Err(de::Error::custom(
                "expected object or string for NativeHint",
            )),
        }
    }
}

/// Hole reason (V4 uses wrapper object format)
///
/// Specific reason why a Hole exists in the IR.
/// Note: Draft is handled separately in Incompleteness, not here.
#[derive(Debug, Clone, PartialEq, JsonSchema)]
pub enum HoleReason {
    /// Reference to something that doesn't exist or was deleted
    UnresolvedReference { target: String },
    /// Reference was deleted during a refactoring operation
    DeletedDuringRefactor {
        #[serde(rename = "tx-id")]
        tx_id: String,
    },
    /// Type mismatch error
    TypeMismatch { expected: String, found: String },
}

impl Serialize for HoleReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            HoleReason::UnresolvedReference { target } => map.serialize_entry(
                "UnresolvedReference",
                &serde_json::json!({ "target": target }),
            )?,
            HoleReason::DeletedDuringRefactor { tx_id } => map.serialize_entry(
                "DeletedDuringRefactor",
                &serde_json::json!({ "tx-id": tx_id }),
            )?,
            HoleReason::TypeMismatch { expected, found } => map.serialize_entry(
                "TypeMismatch",
                &serde_json::json!({ "expected": expected, "found": found }),
            )?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for HoleReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match &value {
            serde_json::Value::Object(map) => {
                if let Some((key, content)) = map.iter().next() {
                    match key.as_str() {
                        "UnresolvedReference" => {
                            let target = content
                                .get("target")
                                .and_then(|t| t.as_str())
                                .ok_or_else(|| de::Error::missing_field("target"))?
                                .to_string();
                            Ok(HoleReason::UnresolvedReference { target })
                        }
                        "DeletedDuringRefactor" => {
                            let tx_id = content
                                .get("tx-id")
                                .and_then(|t| t.as_str())
                                .ok_or_else(|| de::Error::missing_field("tx-id"))?
                                .to_string();
                            Ok(HoleReason::DeletedDuringRefactor { tx_id })
                        }
                        "TypeMismatch" => {
                            let expected = content
                                .get("expected")
                                .and_then(|t| t.as_str())
                                .ok_or_else(|| de::Error::missing_field("expected"))?
                                .to_string();
                            let found = content
                                .get("found")
                                .and_then(|t| t.as_str())
                                .ok_or_else(|| de::Error::missing_field("found"))?
                                .to_string();
                            Ok(HoleReason::TypeMismatch { expected, found })
                        }
                        _ => Err(de::Error::unknown_variant(
                            key,
                            &[
                                "UnresolvedReference",
                                "DeletedDuringRefactor",
                                "TypeMismatch",
                            ],
                        )),
                    }
                } else {
                    Err(de::Error::custom("empty object for HoleReason"))
                }
            }
            _ => Err(de::Error::custom("expected object for HoleReason")),
        }
    }
}

/// Access control
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Access {
    Public,
    Private,
}

/// Entry points for Application distribution
pub type EntryPoints = IndexMap<String, EntryPoint>;

/// Entry point definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EntryPoint {
    pub target: String, // FQName as canonical string
    pub kind: EntryPointKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Entry point kind
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EntryPointKind {
    Main,
    Command,
    Handler,
    Job,
    Policy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_version_string() {
        let v = FormatVersion::String("4.0.0".to_string());
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"4.0.0\"");
    }

    #[test]
    fn test_format_version_integer() {
        let v = FormatVersion::Integer(4);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "4");
    }

    #[test]
    fn test_distribution_library_serialization() {
        let dist = Distribution::Library(LibraryContent {
            package_name: PackageName::new(Path::new("my/pkg")),
            dependencies: IndexMap::new(),
            def: PackageDefinition {
                modules: IndexMap::new(),
            },
        });
        let json = serde_json::to_string(&dist).unwrap();
        assert!(json.contains("\"Library\""));
        assert!(json.contains("packageName"));
    }

    #[test]
    fn test_native_hint_wrapper_format() {
        let hint = NativeHint::Arithmetic;
        let json = serde_json::to_string(&hint).unwrap();
        assert!(json.contains("\"Arithmetic\""));
        assert!(json.contains("{}"));
    }

    #[test]
    fn test_hole_reason_wrapper_format() {
        let reason = HoleReason::TypeMismatch {
            expected: "Int".to_string(),
            found: "String".to_string(),
        };
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("\"TypeMismatch\""));
        assert!(json.contains("\"expected\""));
        assert!(json.contains("\"found\""));
    }

    #[test]
    fn test_incompleteness_draft() {
        let incomp = Incompleteness::Draft;
        let json = serde_json::to_string(&incomp).unwrap();
        assert!(json.contains("\"Draft\""));
        assert!(json.contains("{}"));
    }

    #[test]
    fn test_incompleteness_hole() {
        let incomp = Incompleteness::Hole {
            reason: HoleReason::UnresolvedReference {
                target: "my/pkg:mod#func".to_string(),
            },
        };
        let json = serde_json::to_string(&incomp).unwrap();
        assert!(json.contains("\"Hole\""));
        assert!(json.contains("\"reason\""));
    }

    #[test]
    fn test_native_hint_platform_specific() {
        let hint = NativeHint::PlatformSpecific {
            platform: "wasm".to_string(),
        };
        let json = serde_json::to_string(&hint).unwrap();
        assert!(json.contains("\"PlatformSpecific\""));
        assert!(json.contains("\"platform\""));
        assert!(json.contains("wasm"));
    }

    #[test]
    fn test_hole_reason_with_target() {
        let reason = HoleReason::UnresolvedReference {
            target: "my/pkg:mod#func".to_string(),
        };
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("\"UnresolvedReference\""));
        assert!(json.contains("\"target\""));
        assert!(json.contains("my/pkg:mod#func"));
    }

    #[test]
    fn test_value_body_expression_wrapper() {
        use crate::ir::attributes::ValueAttributes;
        let body = ValueBody::ExpressionBody {
            body: Value::Unit(ValueAttributes::default()),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ExpressionBody\""));
        assert!(json.contains("\"body\""));
    }
}
