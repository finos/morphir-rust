use super::module::{ModuleEntry, ModuleSpecification};
use super::naming::Path;
use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeTuple, Serializer};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Package specification - contains a list of module specifications
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageSpecification<A> {
    pub modules: Vec<ModuleSpecEntry<A>>,
}

/// Module specification entry - [modulePath, ModuleSpecification]
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleSpecEntry<A> {
    pub path: Path,
    pub specification: ModuleSpecification<A>,
}

impl<A: Serialize> Serialize for ModuleSpecEntry<A> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&self.path)?;
        tuple.serialize_element(&self.specification)?;
        tuple.end()
    }
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

/// The message a Specs distribution gives for a module member a specification does not hold.
const SPECS_HOLD_SPECIFICATIONS: &str =
    "a v3 Specs distribution holds module specifications, not definitions";

/// One of a v3 `Specs` distribution's own modules, read strictly.
///
/// [`ModuleSpecEntry`] reads a specification leniently: `types` and `values` default to empty
/// and unknown members are ignored, so a module definition (`{"access": …, "value": …}`) would
/// read as an empty specification and lose its content. A `Specs` distribution's own modules are
/// read through this type instead, which accepts only `types`, `values` and `doc`.
/// Dependencies keep the lenient reading.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecsModuleEntry<A>(pub ModuleSpecEntry<A>);

impl<A> From<SpecsModuleEntry<A>> for ModuleSpecEntry<A> {
    fn from(entry: SpecsModuleEntry<A>) -> Self {
        entry.0
    }
}

impl<'de, A: Deserialize<'de>> Deserialize<'de> for SpecsModuleEntry<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EntryVisitor<A>(std::marker::PhantomData<A>);

        impl<'de, A: Deserialize<'de>> Visitor<'de> for EntryVisitor<A> {
            type Value = SpecsModuleEntry<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a Specs module array [path, specification]")
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let path = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let StrictModuleSpecification(specification) = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                if let Some(IgnoredAny) = seq.next_element()? {
                    return Err(de::Error::custom("Expected end of ModuleSpecEntry array"));
                }
                Ok(SpecsModuleEntry(ModuleSpecEntry {
                    path,
                    specification,
                }))
            }
        }

        deserializer.deserialize_seq(EntryVisitor(std::marker::PhantomData))
    }
}

/// A [`ModuleSpecification`] that refuses every member but `types`, `values` and `doc`.
struct StrictModuleSpecification<A>(ModuleSpecification<A>);

impl<'de, A: Deserialize<'de>> Deserialize<'de> for StrictModuleSpecification<A> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SpecificationVisitor<A>(std::marker::PhantomData<A>);

        impl<'de, A: Deserialize<'de>> Visitor<'de> for SpecificationVisitor<A> {
            type Value = StrictModuleSpecification<A>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a module specification object {types, values, doc}")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut types = None;
                let mut values = None;
                let mut doc = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "types" if types.is_none() => types = Some(map.next_value()?),
                        "values" if values.is_none() => values = Some(map.next_value()?),
                        "doc" if doc.is_none() => doc = Some(map.next_value()?),
                        "types" | "values" | "doc" => {
                            return Err(de::Error::custom(format!("duplicate field `{key}`")));
                        }
                        _ => {
                            return Err(de::Error::custom(format!(
                                "{SPECS_HOLD_SPECIFICATIONS}: unexpected member `{key}`"
                            )));
                        }
                    }
                }
                Ok(StrictModuleSpecification(ModuleSpecification {
                    types: types.unwrap_or_default(),
                    values: values.unwrap_or_default(),
                    doc: doc.flatten(),
                }))
            }
        }

        deserializer.deserialize_map(SpecificationVisitor(std::marker::PhantomData))
    }
}

/// Package definition - contains a list of module entries (full implementation)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageDefinition<TA, VA> {
    pub modules: Vec<ModuleEntry<TA, VA>>,
}
