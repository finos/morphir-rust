use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{AvroField, AvroFullName, AvroType, Properties};
use crate::{AvroDiagnostic, naming::is_valid_identifier};

/// A public Morphir type projected as an Avro artifact root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroRoot {
    pub(super) source_fqname: String,
    pub(super) full_name: AvroFullName,
    pub(super) tpe: AvroType,
    pub(super) doc: Option<String>,
    pub(super) properties: Properties,
    pub(super) referenced_named_declarations: Vec<AvroFullName>,
}

impl AvroRoot {
    pub(crate) fn new(
        source_fqname: String,
        full_name: AvroFullName,
        tpe: AvroType,
        doc: Option<String>,
    ) -> Result<Self, AvroDiagnostic> {
        if source_fqname.is_empty() {
            return Err(AvroDiagnostic::unsupported_morphir_type(
                "empty source FQName",
            ));
        }
        Ok(Self {
            properties: BTreeMap::from([(
                "morphir.fqname".to_owned(),
                Value::String(source_fqname.clone()),
            )]),
            source_fqname,
            full_name,
            tpe,
            doc,
            referenced_named_declarations: Vec::new(),
        })
    }

    /// Return the exact canonical Morphir source FQName.
    pub fn source_fqname(&self) -> &str {
        &self.source_fqname
    }

    /// Return the deterministic artifact identity for this root.
    pub fn full_name(&self) -> &AvroFullName {
        &self.full_name
    }

    /// Return the projected root type.
    pub fn tpe(&self) -> &AvroType {
        &self.tpe
    }

    /// Return source documentation for this artifact root.
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// Return custom root properties.
    pub fn properties(&self) -> &Properties {
        &self.properties
    }

    /// Find a custom root property.
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties.get(name)
    }

    /// Return the named declarations transitively required by this root.
    pub fn referenced_named_declarations(&self) -> &[AvroFullName] {
        &self.referenced_named_declarations
    }
}

/// A checked Avro protocol request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroRequest {
    pub(super) fields: Vec<AvroField>,
}

impl AvroRequest {
    pub(crate) fn new(fields: Vec<AvroField>) -> Result<Self, AvroDiagnostic> {
        let mut names = BTreeSet::new();
        for field in &fields {
            if !names.insert(field.name()) {
                return Err(AvroDiagnostic::name_collision(field.name()));
            }
        }
        Ok(Self { fields })
    }

    /// Return request fields in Morphir signature order.
    pub fn fields(&self) -> &[AvroField] {
        &self.fields
    }
}

/// A checked Avro protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroMessage {
    pub(super) name: String,
    pub(super) request: AvroRequest,
    pub(super) response: AvroType,
    pub(super) errors: Vec<AvroType>,
    pub(super) properties: Properties,
}

impl AvroMessage {
    pub(crate) fn new(
        name: String,
        request: AvroRequest,
        response: AvroType,
        errors: Vec<AvroType>,
        properties: Properties,
    ) -> Result<Self, AvroDiagnostic> {
        if !is_valid_identifier(&name) {
            return Err(AvroDiagnostic::name_collision(format!(
                "invalid Avro message name {name}"
            )));
        }
        Ok(Self {
            name,
            request,
            response,
            errors,
            properties,
        })
    }

    /// Return the message name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the message request.
    pub fn request(&self) -> &AvroRequest {
        &self.request
    }

    /// Return the message response type.
    pub fn response(&self) -> &AvroType {
        &self.response
    }

    /// Return declared Avro protocol errors.
    pub fn errors(&self) -> &[AvroType] {
        &self.errors
    }

    /// Return custom message properties.
    pub fn properties(&self) -> &Properties {
        &self.properties
    }

    /// Find a custom message property.
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties.get(name)
    }
}

/// A renderer-neutral Avro protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protocol {
    pub(super) full_name: AvroFullName,
    pub(super) messages: Vec<AvroMessage>,
    pub(super) properties: Properties,
    pub(super) type_roots: Vec<AvroType>,
    pub(super) referenced_named_declarations: Vec<AvroFullName>,
}

impl Protocol {
    pub(crate) fn new(
        full_name: AvroFullName,
        mut messages: Vec<AvroMessage>,
        type_roots: Vec<AvroType>,
        properties: Properties,
    ) -> Result<Self, AvroDiagnostic> {
        messages.sort_by(|left, right| left.name.cmp(&right.name));
        for pair in messages.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(AvroDiagnostic::name_collision(&pair[0].name));
            }
        }
        Ok(Self {
            full_name,
            messages,
            properties,
            type_roots,
            referenced_named_declarations: Vec::new(),
        })
    }

    /// Return the protocol full name.
    pub fn full_name(&self) -> &AvroFullName {
        &self.full_name
    }

    /// Return custom protocol properties.
    pub fn properties(&self) -> &Properties {
        &self.properties
    }

    /// Return messages in deterministic name order.
    pub fn messages(&self) -> &[AvroMessage] {
        &self.messages
    }

    /// Find a message by its normalized Avro name.
    pub fn message(&self, name: &str) -> Option<&AvroMessage> {
        self.messages.iter().find(|message| message.name == name)
    }

    /// Return the named declarations transitively required by this protocol.
    pub fn referenced_named_declarations(&self) -> &[AvroFullName] {
        &self.referenced_named_declarations
    }
}
