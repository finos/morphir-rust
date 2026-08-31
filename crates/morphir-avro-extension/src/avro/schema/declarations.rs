use serde_json::Value;
use std::collections::BTreeSet;

use super::{AvroFullName, AvroType, Properties};
use crate::{AvroDiagnostic, naming::is_valid_identifier};

/// A field in an Avro record or protocol request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroField {
    name: String,
    tpe: AvroType,
    properties: Properties,
}

impl AvroField {
    pub(crate) fn new(
        name: String,
        tpe: AvroType,
        properties: Properties,
    ) -> Result<Self, AvroDiagnostic> {
        if !is_valid_identifier(&name) {
            return Err(AvroDiagnostic::name_collision(format!(
                "invalid Avro field name {name}"
            )));
        }
        Ok(Self {
            name,
            tpe,
            properties,
        })
    }

    /// Return the semantic Avro field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the field type.
    pub fn tpe(&self) -> &AvroType {
        &self.tpe
    }

    /// Return custom field properties.
    pub fn properties(&self) -> &Properties {
        &self.properties
    }
}

/// A checked named Avro record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordSchema {
    full_name: AvroFullName,
    fields: Vec<AvroField>,
    doc: Option<String>,
    properties: Properties,
}

impl RecordSchema {
    pub(crate) fn new(
        full_name: AvroFullName,
        mut fields: Vec<AvroField>,
        doc: Option<String>,
        properties: Properties,
    ) -> Result<Self, AvroDiagnostic> {
        fields.sort_by(|left, right| left.name.cmp(&right.name));
        Self::new_ordered(full_name, fields, doc, properties)
    }

    pub(crate) fn new_ordered(
        full_name: AvroFullName,
        fields: Vec<AvroField>,
        doc: Option<String>,
        properties: Properties,
    ) -> Result<Self, AvroDiagnostic> {
        let mut names = BTreeSet::new();
        for field in &fields {
            if !names.insert(field.name.as_str()) {
                return Err(AvroDiagnostic::name_collision(&field.name));
            }
        }
        Ok(Self {
            full_name,
            fields,
            doc,
            properties,
        })
    }

    /// Return the record full name.
    pub fn full_name(&self) -> &AvroFullName {
        &self.full_name
    }

    /// Return record fields in deterministic order.
    pub fn fields(&self) -> &[AvroField] {
        &self.fields
    }

    /// Return source documentation for this record.
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// Return custom record properties.
    pub fn properties(&self) -> &Properties {
        &self.properties
    }
}

/// A checked named Avro enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumSchema {
    full_name: AvroFullName,
    symbols: Vec<String>,
    doc: Option<String>,
    properties: Properties,
}

impl EnumSchema {
    pub(crate) fn new(
        full_name: AvroFullName,
        mut symbols: Vec<String>,
        doc: Option<String>,
        properties: Properties,
    ) -> Result<Self, AvroDiagnostic> {
        if symbols.is_empty() {
            return Err(AvroDiagnostic::unsupported_morphir_type(format!(
                "empty enum {full_name}"
            )));
        }
        symbols.sort();
        if let Some(invalid) = symbols.iter().find(|symbol| !is_valid_identifier(symbol)) {
            return Err(AvroDiagnostic::name_collision(format!(
                "invalid Avro enum symbol {invalid}"
            )));
        }
        for pair in symbols.windows(2) {
            if pair[0] == pair[1] {
                return Err(AvroDiagnostic::name_collision(&pair[0]));
            }
        }
        Ok(Self {
            full_name,
            symbols,
            doc,
            properties,
        })
    }

    /// Return the enum full name.
    pub fn full_name(&self) -> &AvroFullName {
        &self.full_name
    }

    /// Return enum symbols in deterministic order.
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// Return source documentation for this enum.
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// Return custom enum properties.
    pub fn properties(&self) -> &Properties {
        &self.properties
    }
}

/// A checked named Avro fixed-width byte sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedSchema {
    full_name: AvroFullName,
    size: usize,
    doc: Option<String>,
    properties: Properties,
}

impl FixedSchema {
    #[allow(dead_code, reason = "fixed physical mappings are not yet supported")]
    pub(crate) fn new(
        full_name: AvroFullName,
        size: usize,
        doc: Option<String>,
        properties: Properties,
    ) -> Result<Self, AvroDiagnostic> {
        if size == 0 {
            return Err(AvroDiagnostic::unsupported_morphir_type(format!(
                "zero-sized fixed {full_name}"
            )));
        }
        Ok(Self {
            full_name,
            size,
            doc,
            properties,
        })
    }

    /// Return the fixed declaration full name.
    pub fn full_name(&self) -> &AvroFullName {
        &self.full_name
    }

    /// Return the fixed width in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Return source documentation for this fixed declaration.
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// Return custom fixed properties.
    pub fn properties(&self) -> &Properties {
        &self.properties
    }
}

/// A named Avro schema declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedSchema {
    /// An Avro record.
    Record(RecordSchema),
    /// An Avro enum.
    Enum(EnumSchema),
    /// An Avro fixed declaration.
    Fixed(FixedSchema),
}

impl NamedSchema {
    /// Return this declaration's full name.
    pub fn full_name(&self) -> &AvroFullName {
        match self {
            Self::Record(schema) => schema.full_name(),
            Self::Enum(schema) => schema.full_name(),
            Self::Fixed(schema) => schema.full_name(),
        }
    }

    /// Return source documentation for this named declaration.
    pub fn doc(&self) -> Option<&str> {
        match self {
            Self::Record(schema) => schema.doc(),
            Self::Enum(schema) => schema.doc(),
            Self::Fixed(schema) => schema.doc(),
        }
    }

    /// Find a custom schema property.
    pub fn property(&self, name: &str) -> Option<&Value> {
        match self {
            Self::Record(schema) => schema.properties().get(name),
            Self::Enum(schema) => schema.properties().get(name),
            Self::Fixed(schema) => schema.properties().get(name),
        }
    }

    /// Find a record field, returning `None` for non-record declarations.
    pub fn field(&self, name: &str) -> Option<&AvroField> {
        match self {
            Self::Record(schema) => schema.fields().iter().find(|field| field.name() == name),
            Self::Enum(_) | Self::Fixed(_) => None,
        }
    }
}
