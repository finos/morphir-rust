use std::{collections::BTreeSet, fmt, sync::OnceLock};

use thiserror::Error;

use super::Properties;
use crate::{AvroDiagnostic, naming::is_valid_identifier};

/// A fully qualified semantic Avro name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AvroFullName {
    namespace: String,
    name: String,
}

impl AvroFullName {
    pub(crate) fn new(namespace: String, name: String) -> Result<Self, AvroDiagnostic> {
        if !is_valid_identifier(&name)
            || (!namespace.is_empty()
                && namespace
                    .split('.')
                    .any(|component| !is_valid_identifier(component)))
        {
            return Err(AvroDiagnostic::name_collision(format!(
                "invalid Avro full name {namespace}.{name}"
            )));
        }
        Ok(Self { namespace, name })
    }

    /// Return the dotted Avro namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Return the local semantic Avro name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for AvroFullName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.namespace.is_empty() {
            formatter.write_str(&self.name)
        } else {
            write!(formatter, "{}.{}", self.namespace, self.name)
        }
    }
}

/// A renderer-neutral Avro type expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvroType {
    /// Avro `null`.
    Null,
    /// Avro `boolean`.
    Boolean,
    /// Avro 32-bit `int`.
    Int,
    /// Avro 64-bit `long`.
    Long,
    /// Avro 32-bit `float`.
    Float,
    /// Avro 64-bit `double`.
    Double,
    /// Avro byte sequence.
    Bytes,
    /// Avro Unicode string.
    String,
    /// Avro array with item type and custom properties.
    Array(Box<AvroType>, Properties),
    /// Avro string-keyed map with value type and custom properties.
    Map(Box<AvroType>, Properties),
    /// A checked Avro union.
    Union(AvroUnion),
    /// A reference to a named Avro declaration.
    Named(AvroFullName),
    /// A physical Avro type decorated with a logical type.
    Logical {
        /// Underlying Avro type.
        physical: Box<AvroType>,
        /// Logical type name.
        name: String,
        /// Additional logical-type properties.
        properties: Properties,
    },
    /// A physical Avro type with non-logical custom properties.
    Annotated {
        /// Underlying Avro type.
        physical: Box<AvroType>,
        /// Custom properties.
        properties: Properties,
    },
}

impl AvroType {
    /// Return custom properties directly attached to this type expression.
    pub fn properties(&self) -> &Properties {
        match self {
            Self::Array(_, properties)
            | Self::Map(_, properties)
            | Self::Logical { properties, .. }
            | Self::Annotated { properties, .. } => properties,
            _ => empty_properties(),
        }
    }
}

fn empty_properties() -> &'static Properties {
    static EMPTY: OnceLock<Properties> = OnceLock::new();
    EMPTY.get_or_init(Properties::new)
}

/// A non-empty Avro union whose branch categories are unique and non-nested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroUnion(Vec<AvroType>);

impl AvroUnion {
    /// Validate and construct an Avro union.
    pub fn new(branches: Vec<AvroType>) -> Result<Self, UnionError> {
        if branches.is_empty() {
            return Err(UnionError::Empty);
        }
        let mut keys = BTreeSet::new();
        for branch in &branches {
            let key = union_branch_key(branch)?;
            if !keys.insert(key.clone()) {
                return Err(UnionError::DuplicateBranch(key));
            }
        }
        Ok(Self(branches))
    }

    /// Return union branches in their semantic order.
    pub fn branches(&self) -> &[AvroType] {
        &self.0
    }
}

/// Why a requested Avro union is invalid.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum UnionError {
    /// Avro unions must contain at least one branch.
    #[error("Avro unions cannot be empty")]
    Empty,
    /// Avro unions cannot directly contain another union.
    #[error("Avro unions cannot directly contain another union")]
    NestedUnion,
    /// The same Avro branch category or named full name appeared twice.
    #[error("duplicate Avro union branch: {0}")]
    DuplicateBranch(String),
}

fn union_branch_key(branch: &AvroType) -> Result<String, UnionError> {
    Ok(match branch {
        AvroType::Null => "null".to_owned(),
        AvroType::Boolean => "boolean".to_owned(),
        AvroType::Int => "int".to_owned(),
        AvroType::Long => "long".to_owned(),
        AvroType::Float => "float".to_owned(),
        AvroType::Double => "double".to_owned(),
        AvroType::Bytes => "bytes".to_owned(),
        AvroType::String => "string".to_owned(),
        AvroType::Array(_, _) => "array".to_owned(),
        AvroType::Map(_, _) => "map".to_owned(),
        AvroType::Union(_) => return Err(UnionError::NestedUnion),
        AvroType::Named(name) => format!("named:{name}"),
        AvroType::Logical { physical, .. } | AvroType::Annotated { physical, .. } => {
            union_branch_key(physical)?
        }
    })
}
