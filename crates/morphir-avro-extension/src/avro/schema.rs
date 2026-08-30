use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::OnceLock,
};

use serde_json::Value;
use thiserror::Error;

use crate::{AvroDiagnostic, ProjectedDiagnostic, naming::is_valid_identifier};

/// Deterministically ordered custom properties attached to an Avro model node.
pub type Properties = BTreeMap<String, Value>;

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
        for pair in fields.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(AvroDiagnostic::name_collision(&pair[0].name));
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

/// A public Morphir type projected as an Avro artifact root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroRoot {
    source_fqname: String,
    full_name: AvroFullName,
    tpe: AvroType,
    doc: Option<String>,
    properties: Properties,
    referenced_named_declarations: Vec<AvroFullName>,
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
    fields: Vec<AvroField>,
}

impl AvroRequest {
    pub(crate) fn new(mut fields: Vec<AvroField>) -> Result<Self, AvroDiagnostic> {
        fields.sort_by(|left, right| left.name.cmp(&right.name));
        for pair in fields.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(AvroDiagnostic::name_collision(&pair[0].name));
            }
        }
        Ok(Self { fields })
    }

    /// Return request fields in deterministic name order.
    pub fn fields(&self) -> &[AvroField] {
        &self.fields
    }
}

/// A checked Avro protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroMessage {
    name: String,
    request: AvroRequest,
    response: AvroType,
    errors: Vec<AvroType>,
    properties: Properties,
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
    full_name: AvroFullName,
    messages: Vec<AvroMessage>,
    properties: Properties,
    type_roots: Vec<AvroType>,
    referenced_named_declarations: Vec<AvroFullName>,
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

/// A complete renderer-neutral Avro projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvroPackage {
    roots: Vec<AvroRoot>,
    schemas: Vec<NamedSchema>,
    linked_schemas: Vec<NamedSchema>,
    protocols: Vec<Protocol>,
    diagnostics: Vec<ProjectedDiagnostic>,
}

impl AvroPackage {
    pub(crate) fn new(
        mut roots: Vec<AvroRoot>,
        schemas: Vec<NamedSchema>,
        linked_schemas: Vec<NamedSchema>,
        protocols: Vec<Protocol>,
        diagnostics: Vec<ProjectedDiagnostic>,
    ) -> Result<Self, AvroDiagnostic> {
        roots.sort_by(|left, right| left.full_name.cmp(&right.full_name));
        let mut source_names = BTreeSet::new();
        for root in &roots {
            if !source_names.insert(root.source_fqname.as_str()) {
                return Err(AvroDiagnostic::name_collision(&root.source_fqname));
            }
        }
        for pair in roots.windows(2) {
            if pair[0].full_name == pair[1].full_name {
                return Err(AvroDiagnostic::name_collision(&pair[0].full_name));
            }
        }
        let mut schemas = schemas;
        schemas.sort_by(|left, right| left.full_name().cmp(right.full_name()));
        for pair in schemas.windows(2) {
            if pair[0].full_name() == pair[1].full_name() {
                return Err(AvroDiagnostic::name_collision(pair[0].full_name()));
            }
        }
        let mut linked_schemas = linked_schemas;
        linked_schemas.sort_by(|left, right| left.full_name().cmp(right.full_name()));
        for pair in linked_schemas.windows(2) {
            if pair[0].full_name() == pair[1].full_name() {
                return Err(AvroDiagnostic::name_collision(pair[0].full_name()));
            }
        }
        let owned_names = schemas
            .iter()
            .map(|schema| schema.full_name().to_string())
            .collect::<BTreeSet<_>>();
        if let Some(duplicate) = linked_schemas
            .iter()
            .find(|schema| owned_names.contains(&schema.full_name().to_string()))
        {
            return Err(AvroDiagnostic::name_collision(duplicate.full_name()));
        }
        let mut protocols = protocols;
        protocols.sort_by(|left, right| left.full_name.cmp(&right.full_name));
        for pair in protocols.windows(2) {
            if pair[0].full_name == pair[1].full_name {
                return Err(AvroDiagnostic::name_collision(&pair[0].full_name));
            }
        }
        let available_schemas = schemas
            .iter()
            .chain(&linked_schemas)
            .map(|schema| (schema.full_name().to_string(), schema))
            .collect::<BTreeMap<_, _>>();
        let mut all_references = BTreeSet::new();
        for schema in available_schemas.values() {
            if let NamedSchema::Record(record) = schema {
                for field in record.fields() {
                    collect_references(field.tpe(), &available_schemas, &mut all_references);
                }
            }
        }
        for root in &mut roots {
            root.referenced_named_declarations = references_for_root(&root.tpe, &available_schemas);
            all_references.extend(root.referenced_named_declarations.iter().cloned());
        }
        for protocol in &mut protocols {
            protocol.referenced_named_declarations =
                references_for_protocol(protocol, &available_schemas);
            all_references.extend(protocol.referenced_named_declarations.iter().cloned());
        }
        if let Some(unresolved) = all_references
            .iter()
            .find(|name| !available_schemas.contains_key(&name.to_string()))
        {
            return Err(AvroDiagnostic::missing_linked_dependency(unresolved));
        }
        Ok(Self {
            roots,
            schemas,
            linked_schemas,
            protocols,
            diagnostics,
        })
    }

    /// Return projected public type roots in deterministic full-name order.
    pub fn roots(&self) -> &[AvroRoot] {
        &self.roots
    }

    /// Find a projected root by exact Morphir source FQName.
    pub fn root(&self, source_fqname: &str) -> Option<&AvroRoot> {
        self.roots
            .iter()
            .find(|root| root.source_fqname == source_fqname)
    }

    /// Return named schema declarations in deterministic full-name order.
    pub fn schemas(&self) -> &[NamedSchema] {
        &self.schemas
    }

    /// Return reachable linked declarations in deterministic full-name order.
    ///
    /// Renderers emit these declarations as separate linked artifacts or IDL
    /// imports rather than embedding them in an owned root or protocol.
    pub fn linked_schemas(&self) -> &[NamedSchema] {
        &self.linked_schemas
    }

    /// Return projected protocols.
    pub fn protocols(&self) -> &[Protocol] {
        &self.protocols
    }

    /// Find a projected protocol by dotted Avro full name.
    pub fn protocol(&self, full_name: &str) -> Option<&Protocol> {
        self.protocols
            .iter()
            .find(|protocol| protocol.full_name.to_string() == full_name)
    }

    /// Return the only protocol, or `None` when the package has zero or many.
    pub fn only_protocol(&self) -> Option<&Protocol> {
        (self.protocols.len() == 1).then(|| &self.protocols[0])
    }

    /// Return non-fatal projection diagnostics.
    pub fn diagnostics(&self) -> &[ProjectedDiagnostic] {
        &self.diagnostics
    }

    /// Find a named schema declaration by dotted Avro full name.
    pub fn named_schema(&self, full_name: &str) -> Option<&NamedSchema> {
        self.schemas
            .iter()
            .find(|schema| schema.full_name().to_string() == full_name)
    }

    /// Find a reachable linked declaration by dotted Avro full name.
    pub fn linked_schema(&self, full_name: &str) -> Option<&NamedSchema> {
        self.linked_schemas
            .iter()
            .find(|schema| schema.full_name().to_string() == full_name)
    }
}

fn references_for_root(
    tpe: &AvroType,
    schemas: &BTreeMap<String, &NamedSchema>,
) -> Vec<AvroFullName> {
    let mut references = BTreeSet::new();
    collect_references(tpe, schemas, &mut references);
    references.into_iter().collect()
}

fn references_for_protocol(
    protocol: &Protocol,
    schemas: &BTreeMap<String, &NamedSchema>,
) -> Vec<AvroFullName> {
    let mut references = BTreeSet::new();
    for root in &protocol.type_roots {
        collect_references(root, schemas, &mut references);
    }
    for message in &protocol.messages {
        for field in message.request.fields() {
            collect_references(field.tpe(), schemas, &mut references);
        }
        collect_references(&message.response, schemas, &mut references);
        for error in &message.errors {
            collect_references(error, schemas, &mut references);
        }
    }
    references.into_iter().collect()
}

fn collect_references(
    tpe: &AvroType,
    schemas: &BTreeMap<String, &NamedSchema>,
    references: &mut BTreeSet<AvroFullName>,
) {
    match tpe {
        AvroType::Array(element, _)
        | AvroType::Map(element, _)
        | AvroType::Logical {
            physical: element, ..
        }
        | AvroType::Annotated {
            physical: element, ..
        } => {
            collect_references(element, schemas, references);
        }
        AvroType::Union(union) => {
            for branch in union.branches() {
                collect_references(branch, schemas, references);
            }
        }
        AvroType::Named(name) if references.insert(name.clone()) => {
            if let Some(NamedSchema::Record(record)) = schemas.get(&name.to_string()) {
                for field in record.fields() {
                    collect_references(field.tpe(), schemas, references);
                }
            }
        }
        AvroType::Null
        | AvroType::Boolean
        | AvroType::Int
        | AvroType::Long
        | AvroType::Float
        | AvroType::Double
        | AvroType::Bytes
        | AvroType::String
        | AvroType::Named(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_named_full_names_are_rejected_by_unions() {
        let name = AvroFullName::new("acme".to_owned(), "Customer".to_owned()).unwrap();
        assert_eq!(
            AvroUnion::new(vec![AvroType::Named(name.clone()), AvroType::Named(name)]),
            Err(UnionError::DuplicateBranch(
                "named:acme.Customer".to_owned()
            ))
        );
    }

    #[test]
    fn packages_reject_unresolved_root_and_schema_references() {
        let missing = AvroFullName::new("acme.missing".to_owned(), "Type".to_owned()).unwrap();
        let root = AvroRoot::new(
            "acme/customer:domain#root".to_owned(),
            AvroFullName::new("acme.customer.domain".to_owned(), "Root".to_owned()).unwrap(),
            AvroType::Named(missing.clone()),
            None,
        )
        .unwrap();
        let error = AvroPackage::new(vec![root], Vec::new(), Vec::new(), Vec::new(), Vec::new())
            .unwrap_err();
        assert_eq!(error.code(), "AVRO006");
        assert_eq!(
            error.message(),
            "missing linked dependency: acme.missing.Type"
        );

        let schema = NamedSchema::Record(
            RecordSchema::new(
                AvroFullName::new("acme.customer".to_owned(), "Wrapper".to_owned()).unwrap(),
                vec![
                    AvroField::new(
                        "value".to_owned(),
                        AvroType::Named(missing),
                        Properties::new(),
                    )
                    .unwrap(),
                ],
                None,
                Properties::new(),
            )
            .unwrap(),
        );
        let error = AvroPackage::new(Vec::new(), vec![schema], Vec::new(), Vec::new(), Vec::new())
            .unwrap_err();
        assert_eq!(error.code(), "AVRO006");
        assert!(error.message().contains("acme.missing.Type"));
    }
}
