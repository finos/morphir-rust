//! Classic IR Module types
//!
//! Module structures for the Classic Morphir IR format.

use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

use super::access::AccessControlled;
use super::documented::Documented;
use super::naming::{Name, Path};
use super::types::TypeDefinition;
use super::value::ValueDefinition;

/// Module entry - [modulePath, AccessControlled<ModuleDefinition>]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleEntry<TA, VA> {
    pub path: Path,
    pub definition: AccessControlled<ModuleDefinition<TA, VA>>,
}

impl<'de, TA: Deserialize<'de>, VA: Deserialize<'de>> Deserialize<'de> for ModuleEntry<TA, VA> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ModuleEntryVisitor<TA, VA>(std::marker::PhantomData<(TA, VA)>);

        impl<'de, TA: Deserialize<'de>, VA: Deserialize<'de>> Visitor<'de> for ModuleEntryVisitor<TA, VA> {
            type Value = ModuleEntry<TA, VA>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a ModuleEntry array [path, definition]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let path = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let definition = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                if let Some(IgnoredAny) = seq.next_element()? {
                    return Err(de::Error::custom("Expected end of ModuleEntry array"));
                }

                Ok(ModuleEntry { path, definition })
            }
        }

        deserializer.deserialize_seq(ModuleEntryVisitor(std::marker::PhantomData))
    }
}

/// Module specification (public interface only)
pub type ModuleTypeSpecification<A> = (Name, Documented<super::types::TypeSpecification<A>>);
pub type ModuleValueSpecification<A> = (Name, Documented<super::value::ValueSpecification<A>>);
pub type ModuleTypeDefinition<A> = (Name, AccessControlled<Documented<TypeDefinition<A>>>);
pub type ModuleValueDefinition<TA, VA> =
    (Name, AccessControlled<Documented<ValueDefinition<TA, VA>>>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "A: Deserialize<'de>"))]
pub struct ModuleSpecification<A> {
    #[serde(default)]
    pub types: Vec<ModuleTypeSpecification<A>>,
    #[serde(default)]
    pub values: Vec<ModuleValueSpecification<A>>,
    pub doc: Option<String>,
}

/// Module definition (full implementation)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(bound(deserialize = "TA: Deserialize<'de>, VA: Deserialize<'de>"))]
pub struct ModuleDefinition<TA, VA> {
    #[serde(default)]
    pub types: Vec<ModuleTypeDefinition<TA>>,
    #[serde(default)]
    pub values: Vec<ModuleValueDefinition<TA, VA>>,
    pub doc: Option<String>,
}
