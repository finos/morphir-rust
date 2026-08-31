use std::collections::{BTreeMap, BTreeSet};

use super::{AvroFullName, AvroRoot, AvroType, NamedSchema, Protocol};
use crate::{AvroDiagnostic, ProjectedDiagnostic};

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

    pub(crate) fn named_root_is_carried_by_a_protocol(&self, root: &AvroRoot) -> bool {
        let AvroType::Named(name) = root.tpe() else {
            return false;
        };
        self.protocols
            .iter()
            .any(|protocol| protocol.referenced_named_declarations().contains(name))
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
