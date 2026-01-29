use super::module::{ModuleEntry, ModuleSpecification};
use super::naming::Path;
use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Package specification - contains a list of module specifications
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageSpecification<A> {
    pub modules: Vec<ModuleSpecEntry<A>>,
}

/// Module specification entry - [modulePath, ModuleSpecification]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleSpecEntry<A> {
    pub path: Path,
    pub specification: ModuleSpecification<A>,
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for ModuleSpecEntry<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ModuleSpecEntryVisitor<A>(std::marker::PhantomData<A>);

        impl<'de, A: Deserialize<'de>> Visitor<'de> for ModuleSpecEntryVisitor<A> {
            type Value = ModuleSpecEntry<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a ModuleSpecEntry array [path, specification]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let path = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let specification = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;

                if let Some(IgnoredAny) = seq.next_element()? {
                    return Err(de::Error::custom("Expected end of ModuleSpecEntry array"));
                }

                Ok(ModuleSpecEntry {
                    path,
                    specification,
                })
            }
        }

        deserializer.deserialize_seq(ModuleSpecEntryVisitor(std::marker::PhantomData))
    }
}

/// Package definition - contains a list of module entries (full implementation)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageDefinition<TA, VA> {
    pub modules: Vec<ModuleEntry<TA, VA>>,
}

/// Alias for PackageDefinition for backward compatibility
pub type Package<TA, VA> = PackageDefinition<TA, VA>;
