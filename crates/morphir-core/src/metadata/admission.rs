//! Semantic admission of expanded facts using declarations supplied by the caller.
//!
//! A reader can retain an expanded fact before its declaration is available.
//! Admission establishes when it has validated meaning; this module does not
//! acquire or authenticate the declaration provider.

use super::{Carrier, Fact, ObjectTerm};
use crate::data_value::{DataValueError, DataValueValidator};
use crate::ir::v4;
use crate::naming::FQName;
use crate::node_address::{NodeRoot, NodeUri};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Semantic role of an addressed subject in its supplying IR artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubjectRole {
    TypeSpecification,
    TypeDefinition,
    ValueSpecification,
    ValueDefinition,
    TypeExpression,
    ValueExpression,
    Pattern,
    Module,
    Package,
}

/// The declaration's representation of one expanded object.
#[derive(Debug, Clone)]
pub enum ObjectDeclaration {
    Data(v4::Type),
    Json {
        datatype: NodeUri,
        ty: DeclaredDataType,
    },
    Node(NodeTargetKind),
}

impl ObjectDeclaration {
    pub fn data(ty: v4::Type) -> Self {
        Self::Data(ty)
    }

    /// One structured `@json` object checked against a V4 type.
    pub fn json(datatype: NodeUri, ty: v4::Type) -> Self {
        Self::Json {
            datatype,
            ty: DeclaredDataType::V4(Box::new(ty)),
        }
    }

    /// A V3 sidecar entry point's closed type, projected as `@json`.
    pub fn sidecar_json(datatype: NodeUri, entry_point: FQName) -> Self {
        Self::Json {
            datatype,
            ty: DeclaredDataType::V3(entry_point),
        }
    }

    pub fn node(target: NodeTargetKind) -> Self {
        Self::Node(target)
    }
}

#[derive(Debug, Clone)]
pub enum DeclaredDataType {
    V3(FQName),
    V4(Box<v4::Type>),
}

/// The resolved target's broad IR node kind, supplied by the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTargetKind {
    Type,
    Value,
    Module,
    Package,
}

/// A semantic interpreter supported by this bounded admission layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpreter {
    TargetNameLanguageIds,
}

/// A predicate's semantic interpretation after its type has been checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpretation {
    Descriptive,
    Required(Interpreter),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationRole {
    ValueSpecification,
    SidecarTypeSpecification,
}

/// One verified predicate supplied by an external declaration closure.
#[derive(Debug, Clone)]
pub struct PredicateDeclaration {
    uri: NodeUri,
    role: DeclarationRole,
    object: ObjectDeclaration,
    subjects: BTreeSet<SubjectRole>,
    interpretation: Interpretation,
}

impl PredicateDeclaration {
    pub fn value(
        uri: NodeUri,
        object: ObjectDeclaration,
        subjects: impl IntoIterator<Item = SubjectRole>,
        interpretation: Interpretation,
    ) -> Self {
        Self {
            uri,
            role: DeclarationRole::ValueSpecification,
            object,
            subjects: subjects.into_iter().collect(),
            interpretation,
        }
    }

    /// One legacy decorator type specification admitted only through its sidecar.
    pub fn sidecar_type(
        uri: NodeUri,
        object: ObjectDeclaration,
        subjects: impl IntoIterator<Item = SubjectRole>,
    ) -> Self {
        Self {
            uri,
            role: DeclarationRole::SidecarTypeSpecification,
            object,
            subjects: subjects.into_iter().collect(),
            interpretation: Interpretation::Descriptive,
        }
    }

    /// The declared datatype needed by `@json` context expansion.
    pub fn json_datatype(&self) -> Option<&NodeUri> {
        match &self.object {
            ObjectDeclaration::Json { datatype, .. } => Some(datatype),
            _ => None,
        }
    }
}

/// A fact's semantic validation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Validated,
    PreservedUnvalidated(UnvalidatedReason),
}

/// Why an expanded fact remains available for roundtrip but lacks validated meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnvalidatedReason {
    PredicateDeclaration,
    Target,
    RequiredInterpreter(Interpreter),
}

/// A declaration or expanded fact violates the supplied closure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("predicate URI root does not match its declaration role")]
    InvalidDeclarationRole,
    #[error("predicate closure contains a duplicate declaration")]
    DuplicatePredicate,
    #[error("@json datatype must name a type declaration")]
    InvalidDatatypeDeclaration,
    #[error("predicate declaration must allow at least one subject role")]
    EmptySubjectRoles,
    #[error("node-local carrier does not match the fact subject")]
    CarrierSubjectMismatch,
    #[error("type-specification predicate requires its matching sidecar carrier")]
    DeclarationCarrierMismatch,
    #[error("fact subject role is not allowed by predicate declaration")]
    SubjectRoleMismatch,
    #[error("predicate object does not match its declared object kind")]
    ObjectKindMismatch,
    #[error("typed JSON datatype does not match its predicate declaration")]
    DatatypeMismatch,
    #[error("node reference target kind does not match its predicate declaration")]
    NodeTargetKindMismatch,
    #[error("invalid language ID at {path}")]
    InvalidLanguageId { path: String },
    #[error("semantic interpreter received a value with the wrong structure")]
    InterpreterInputMismatch,
    #[error(transparent)]
    DataValue(#[from] DataValueError),
}

/// Predicate declarations bound to the caller's authenticated provider set.
#[derive(Debug, Default)]
pub struct PredicateClosure {
    declarations: BTreeMap<String, PredicateDeclaration>,
}

impl PredicateClosure {
    pub fn new(
        declarations: impl IntoIterator<Item = PredicateDeclaration>,
    ) -> Result<Self, AdmissionError> {
        let mut indexed = BTreeMap::new();
        for declaration in declarations {
            let expected_root = match declaration.role {
                DeclarationRole::ValueSpecification => {
                    matches!(declaration.uri.root(), NodeRoot::Value { .. })
                }
                DeclarationRole::SidecarTypeSpecification => {
                    matches!(declaration.uri.root(), NodeRoot::Type { .. })
                        && matches!(declaration.object, ObjectDeclaration::Json { .. })
                }
            };
            if !expected_root || !declaration.uri.steps().is_empty() {
                return Err(AdmissionError::InvalidDeclarationRole);
            }
            if let ObjectDeclaration::Json { datatype, .. } = &declaration.object
                && (!matches!(datatype.root(), NodeRoot::Type { .. })
                    || !datatype.steps().is_empty())
            {
                return Err(AdmissionError::InvalidDatatypeDeclaration);
            }
            if declaration.subjects.is_empty() {
                return Err(AdmissionError::EmptySubjectRoles);
            }
            let key = declaration.uri.to_string();
            if indexed.insert(key, declaration).is_some() {
                return Err(AdmissionError::DuplicatePredicate);
            }
        }
        Ok(Self {
            declarations: indexed,
        })
    }

    /// Look up the typed JSON declaration for context expansion.
    pub fn json_datatype(&self, predicate: &NodeUri) -> Option<NodeUri> {
        self.declarations
            .get(&predicate.to_string())
            .and_then(PredicateDeclaration::json_datatype)
            .cloned()
    }

    /// Validate a fact after context expansion and subject-role resolution.
    pub fn admit(
        &self,
        fact: &Fact,
        subject: SubjectRole,
        carrier: &Carrier,
        validator: &DataValueValidator,
        available_interpreters: &[Interpreter],
        target_kind: Option<NodeTargetKind>,
    ) -> Result<Admission, AdmissionError> {
        match carrier {
            Carrier::AttributesFacts(uri) | Carrier::AnnotationsFacts(uri)
                if uri != fact.subject() =>
            {
                return Err(AdmissionError::CarrierSubjectMismatch);
            }
            Carrier::Sidecar { target, .. } if target != fact.subject() => {
                return Err(AdmissionError::CarrierSubjectMismatch);
            }
            _ => {}
        }
        let Some(declaration) = self.declarations.get(&fact.predicate().to_string()) else {
            return Ok(Admission::PreservedUnvalidated(
                UnvalidatedReason::PredicateDeclaration,
            ));
        };
        match (declaration.role, carrier) {
            (DeclarationRole::ValueSpecification, Carrier::Sidecar { .. }) => {
                return Err(AdmissionError::DeclarationCarrierMismatch);
            }
            (DeclarationRole::SidecarTypeSpecification, Carrier::Sidecar { entry_point, .. })
                if entry_point == fact.predicate() => {}
            (DeclarationRole::SidecarTypeSpecification, _) => {
                return Err(AdmissionError::DeclarationCarrierMismatch);
            }
            _ => {}
        }
        if !declaration.subjects.contains(&subject) {
            return Err(AdmissionError::SubjectRoleMismatch);
        }
        match (&declaration.object, fact.object()) {
            (ObjectDeclaration::Data(ty), ObjectTerm::Value(value))
                if value.datatype().is_none() =>
            {
                validator.validate_v4_data(ty, value.value())?;
            }
            (ObjectDeclaration::Json { datatype, ty }, ObjectTerm::Value(value)) => {
                if value.datatype() != Some(datatype) {
                    return Err(AdmissionError::DatatypeMismatch);
                }
                match ty {
                    DeclaredDataType::V3(entry_point) => {
                        validator.validate_reference(entry_point, value.value())?;
                    }
                    DeclaredDataType::V4(ty) => {
                        validator.validate_v4_data(ty, value.value())?;
                    }
                }
            }
            (ObjectDeclaration::Node(expected), ObjectTerm::NodeRef(target)) => {
                if !target.steps().is_empty() || !target_root_matches(*expected, target.root()) {
                    return Err(AdmissionError::NodeTargetKindMismatch);
                }
                match target_kind {
                    Some(actual) if actual == *expected => {}
                    Some(_) => return Err(AdmissionError::NodeTargetKindMismatch),
                    None => {
                        return Ok(Admission::PreservedUnvalidated(UnvalidatedReason::Target));
                    }
                }
            }
            _ => return Err(AdmissionError::ObjectKindMismatch),
        }
        if let Interpretation::Required(interpreter) = declaration.interpretation {
            if !available_interpreters.contains(&interpreter) {
                return Ok(Admission::PreservedUnvalidated(
                    UnvalidatedReason::RequiredInterpreter(interpreter),
                ));
            }
            match interpreter {
                Interpreter::TargetNameLanguageIds => {
                    validate_target_name_language_ids(fact.object())?;
                }
            }
        }
        Ok(Admission::Validated)
    }
}

fn target_root_matches(expected: NodeTargetKind, root: &NodeRoot) -> bool {
    matches!(
        (expected, root),
        (NodeTargetKind::Type, NodeRoot::Type { .. })
            | (NodeTargetKind::Value, NodeRoot::Value { .. })
            | (NodeTargetKind::Module, NodeRoot::Module { .. })
            | (NodeTargetKind::Package, NodeRoot::Package)
    )
}

fn validate_target_name_language_ids(object: &ObjectTerm) -> Result<(), AdmissionError> {
    let ObjectTerm::Value(value) = object else {
        return Err(AdmissionError::InterpreterInputMismatch);
    };
    let Some(record) = value.value().as_object() else {
        return Err(AdmissionError::InterpreterInputMismatch);
    };
    for field in ["frontend", "backend"] {
        let Some(languages) = record.get(field).and_then(Value::as_object) else {
            return Err(AdmissionError::InterpreterInputMismatch);
        };
        for language in languages.keys() {
            if !valid_language_id(language) {
                return Err(AdmissionError::InvalidLanguageId {
                    path: format!("$.{field}{}", json_member_path(language)),
                });
            }
        }
    }
    Ok(())
}

fn valid_language_id(value: &str) -> bool {
    let mut segments = value.split('-');
    let Some(first) = segments.next() else {
        return false;
    };
    let mut chars = first.bytes();
    matches!(chars.next(), Some(b'a'..=b'z'))
        && chars.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && segments.all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn json_member_path(member: &str) -> String {
    if member
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && member
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
    {
        format!(".{member}")
    } else {
        format!(
            "[{}]",
            serde_json::to_string(member).expect("JSON member names serialize")
        )
    }
}
