//! Grammar-aware detection of linked metadata in concrete V4 carriers.

use super::{
    AccessControlled, Annotation, AnnotationArgument, Annotations, Distribution, DocumentMeta,
    Documented, MetadataScope, ModuleDefinition, ModuleSpecification, PackageDefinition,
    PackageSpecification, Type, TypeDefinition, TypeSpecification, Value, ValueDefinition,
    ValueSpecification,
};
use crate::metadata::EffectiveContext;
use crate::traversal::IrCursor;
use crate::traversal::v4::{V4Visitor, walk_type, walk_value};

/// True when a concrete V4 fragment owns a linked-metadata carrier.
pub trait LinkedMetadataCarrier {
    fn contains_linked_metadata(&self) -> bool;
}

#[derive(Default)]
struct Detector(bool);

impl Detector {
    fn ty(&mut self, value: &Type) {
        self.visit_type(&mut IrCursor::root(), value);
    }

    fn value(&mut self, value: &Value) {
        self.visit_value(&mut IrCursor::root(), value);
    }

    fn definition(&mut self, value: &ValueDefinition) {
        self.visit_definition(&mut IrCursor::root(), value);
    }
}

impl V4Visitor for Detector {
    fn visit_type(&mut self, cursor: &mut IrCursor, value: &Type) {
        self.0 |= !value.attributes().metadata.is_empty();
        if !self.0 {
            walk_type(self, cursor, value);
        }
    }

    fn visit_value(&mut self, cursor: &mut IrCursor, value: &Value) {
        let attributes = value.attributes();
        self.0 |= !attributes.metadata.is_empty();
        if let Some(inferred) = &attributes.inferred_type {
            self.visit_type(cursor, inferred);
        }
        if let Value::Hole(_, _, Some(expected)) = value {
            self.visit_type(cursor, expected);
        }
        if !self.0 {
            walk_value(self, cursor, value);
        }
    }
}

impl LinkedMetadataCarrier for Type {
    fn contains_linked_metadata(&self) -> bool {
        let mut detector = Detector::default();
        detector.ty(self);
        detector.0
    }
}

impl LinkedMetadataCarrier for Value {
    fn contains_linked_metadata(&self) -> bool {
        let mut detector = Detector::default();
        detector.value(self);
        detector.0
    }
}

impl LinkedMetadataCarrier for ValueDefinition {
    fn contains_linked_metadata(&self) -> bool {
        let mut detector = Detector::default();
        detector.definition(self);
        detector.0
    }
}

impl<T: LinkedMetadataCarrier> LinkedMetadataCarrier for AccessControlled<T> {
    fn contains_linked_metadata(&self) -> bool {
        self.value.contains_linked_metadata()
    }
}

impl<T: LinkedMetadataCarrier> LinkedMetadataCarrier for Documented<T> {
    fn contains_linked_metadata(&self) -> bool {
        self.value.contains_linked_metadata()
    }
}

impl LinkedMetadataCarrier for Annotations {
    fn contains_linked_metadata(&self) -> bool {
        self.metadata.is_some()
            || self.entries.iter().any(|entry| {
                matches!(
                    entry,
                    Annotation::LinkedCompact { .. }
                        | Annotation::LinkedStructured { .. }
                        | Annotation::PendingCompact { .. }
                        | Annotation::PendingStructured { .. }
                ) || match entry {
                    Annotation::Structured { args, .. }
                    | Annotation::LinkedStructured { args, .. }
                    | Annotation::PendingStructured { args, .. } => {
                        args.iter().any(|arg| match arg {
                            AnnotationArgument::Positional(value)
                            | AnnotationArgument::Named { value, .. } => {
                                value.contains_linked_metadata()
                            }
                        })
                    }
                    _ => false,
                }
            })
    }
}

impl LinkedMetadataCarrier for TypeSpecification {
    fn contains_linked_metadata(&self) -> bool {
        match self {
            Self::TypeAliasSpecification {
                annotations,
                type_expr,
                ..
            } => annotations.contains_linked_metadata() || type_expr.contains_linked_metadata(),
            Self::OpaqueTypeSpecification { annotations, .. } => {
                annotations.contains_linked_metadata()
            }
            Self::CustomTypeSpecification {
                annotations,
                constructors,
                ..
            } => {
                annotations.contains_linked_metadata()
                    || constructors.iter().any(|constructor| {
                        constructor
                            .args
                            .iter()
                            .any(|arg| arg.arg_type.contains_linked_metadata())
                    })
            }
            Self::DerivedTypeSpecification {
                annotations,
                base_type,
                ..
            } => annotations.contains_linked_metadata() || base_type.contains_linked_metadata(),
        }
    }
}

impl LinkedMetadataCarrier for TypeDefinition {
    fn contains_linked_metadata(&self) -> bool {
        match self {
            Self::TypeAliasDefinition { type_expr, .. } => type_expr.contains_linked_metadata(),
            Self::CustomTypeDefinition { constructors, .. } => {
                constructors.value.iter().any(|constructor| {
                    constructor
                        .args
                        .iter()
                        .any(|arg| arg.arg_type.contains_linked_metadata())
                })
            }
            Self::IncompleteTypeDefinition {
                incompleteness,
                partial_type_expr,
                ..
            } => {
                partial_type_expr
                    .as_ref()
                    .is_some_and(LinkedMetadataCarrier::contains_linked_metadata)
                    || match incompleteness {
                        super::Incompleteness::Hole { partial_body, .. } => partial_body
                            .as_ref()
                            .is_some_and(LinkedMetadataCarrier::contains_linked_metadata),
                        super::Incompleteness::Draft => false,
                    }
            }
        }
    }
}

impl LinkedMetadataCarrier for ValueSpecification {
    fn contains_linked_metadata(&self) -> bool {
        self.annotations.contains_linked_metadata()
            || self
                .inputs
                .values()
                .any(LinkedMetadataCarrier::contains_linked_metadata)
            || self.output.contains_linked_metadata()
    }
}

impl LinkedMetadataCarrier for ModuleSpecification {
    fn contains_linked_metadata(&self) -> bool {
        self.annotations.contains_linked_metadata()
            || self
                .types
                .values()
                .any(LinkedMetadataCarrier::contains_linked_metadata)
            || self
                .values
                .values()
                .any(LinkedMetadataCarrier::contains_linked_metadata)
    }
}

impl LinkedMetadataCarrier for ModuleDefinition {
    fn contains_linked_metadata(&self) -> bool {
        self.types
            .values()
            .any(LinkedMetadataCarrier::contains_linked_metadata)
            || self
                .values
                .values()
                .any(LinkedMetadataCarrier::contains_linked_metadata)
    }
}

impl LinkedMetadataCarrier for PackageSpecification {
    fn contains_linked_metadata(&self) -> bool {
        self.modules
            .values()
            .any(LinkedMetadataCarrier::contains_linked_metadata)
    }
}

impl LinkedMetadataCarrier for PackageDefinition {
    fn contains_linked_metadata(&self) -> bool {
        self.modules
            .values()
            .any(LinkedMetadataCarrier::contains_linked_metadata)
    }
}

impl LinkedMetadataCarrier for Distribution {
    fn contains_linked_metadata(&self) -> bool {
        match self {
            Self::Library(content) => {
                content.def.contains_linked_metadata()
                    || content
                        .dependencies
                        .values()
                        .any(LinkedMetadataCarrier::contains_linked_metadata)
            }
            Self::Specs(content) => {
                content.spec.contains_linked_metadata()
                    || content
                        .dependencies
                        .values()
                        .any(LinkedMetadataCarrier::contains_linked_metadata)
            }
            Self::Application(content) => {
                content.def.contains_linked_metadata()
                    || content
                        .dependencies
                        .values()
                        .any(LinkedMetadataCarrier::contains_linked_metadata)
            }
        }
    }
}

/// Resolve every local carrier over the document defaults after the entire file is decoded.
pub(super) fn validate_document_scopes(
    distribution: &mut Distribution,
    metadata: Option<&DocumentMeta>,
) -> Result<(), String> {
    let context = metadata
        .map(DocumentMeta::effective_context)
        .unwrap_or_default();
    let mut validator = Validator {
        context,
        error: None,
    };
    validator.distribution(distribution);
    validator.error.map_or(Ok(()), Err)
}

struct Validator {
    context: EffectiveContext,
    error: Option<String>,
}

impl Validator {
    fn scope(&mut self, scope: &MetadataScope) -> Option<EffectiveContext> {
        if self.error.is_some() {
            return None;
        }
        match scope.validate_over(&self.context) {
            Ok(context) => Some(context),
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }

    fn annotations(&mut self, annotations: &mut Annotations) {
        let scope = annotations.metadata.as_deref().cloned().unwrap_or_default();
        let Some(context) = self.scope(&scope) else {
            return;
        };
        for entry in &mut annotations.entries {
            match entry {
                Annotation::PendingCompact { authored_name } => {
                    match context.expand_key(authored_name) {
                        Ok(expanded) => {
                            *entry = Annotation::LinkedCompact {
                                authored_name: authored_name.clone(),
                                declaration: expanded.uri().clone(),
                            };
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
                Annotation::PendingStructured {
                    authored_name,
                    args,
                } => match context.expand_key(authored_name) {
                    Ok(expanded) => {
                        *entry = Annotation::LinkedStructured {
                            authored_name: authored_name.clone(),
                            declaration: expanded.uri().clone(),
                            args: std::mem::take(args),
                        };
                    }
                    Err(error) => self.error = Some(error.to_string()),
                },
                Annotation::LinkedCompact {
                    authored_name,
                    declaration,
                }
                | Annotation::LinkedStructured {
                    authored_name,
                    declaration,
                    ..
                } => match context.expand_key(authored_name) {
                    Ok(expanded) if expanded.uri() == declaration => {}
                    Ok(_) => {
                        self.error =
                            Some("annotation declaration changed under document context".to_owned())
                    }
                    Err(error) => self.error = Some(error.to_string()),
                },
                Annotation::Compact { .. } | Annotation::Structured { .. } => {}
            }
            let args = match entry {
                Annotation::Structured { args, .. } | Annotation::LinkedStructured { args, .. } => {
                    Some(args)
                }
                _ => None,
            };
            if let Some(args) = args {
                for arg in args {
                    match arg {
                        AnnotationArgument::Positional(value)
                        | AnnotationArgument::Named { value, .. } => {
                            self.visit_value(&mut IrCursor::root(), value)
                        }
                    }
                }
            }
        }
    }

    fn type_specification(&mut self, specification: &mut TypeSpecification) {
        match specification {
            TypeSpecification::TypeAliasSpecification {
                annotations,
                type_expr,
                ..
            } => {
                self.annotations(annotations);
                self.visit_type(&mut IrCursor::root(), type_expr);
            }
            TypeSpecification::OpaqueTypeSpecification { annotations, .. } => {
                self.annotations(annotations)
            }
            TypeSpecification::CustomTypeSpecification {
                annotations,
                constructors,
                ..
            } => {
                self.annotations(annotations);
                for constructor in constructors {
                    for arg in &constructor.args {
                        self.visit_type(&mut IrCursor::root(), &arg.arg_type);
                    }
                }
            }
            TypeSpecification::DerivedTypeSpecification {
                annotations,
                base_type,
                ..
            } => {
                self.annotations(annotations);
                self.visit_type(&mut IrCursor::root(), base_type);
            }
        }
    }

    fn type_definition(&mut self, definition: &TypeDefinition) {
        match definition {
            TypeDefinition::TypeAliasDefinition { type_expr, .. } => {
                self.visit_type(&mut IrCursor::root(), type_expr)
            }
            TypeDefinition::CustomTypeDefinition { constructors, .. } => {
                for constructor in &constructors.value {
                    for arg in &constructor.args {
                        self.visit_type(&mut IrCursor::root(), &arg.arg_type);
                    }
                }
            }
            TypeDefinition::IncompleteTypeDefinition {
                incompleteness,
                partial_type_expr,
                ..
            } => {
                if let Some(value) = partial_type_expr {
                    self.visit_type(&mut IrCursor::root(), value);
                }
                if let super::Incompleteness::Hole {
                    partial_body: Some(value),
                    ..
                } = incompleteness
                {
                    self.visit_type(&mut IrCursor::root(), value);
                }
            }
        }
    }

    fn value_specification(&mut self, specification: &mut ValueSpecification) {
        self.annotations(&mut specification.annotations);
        for value in specification.inputs.values() {
            self.visit_type(&mut IrCursor::root(), value);
        }
        self.visit_type(&mut IrCursor::root(), &specification.output);
    }

    fn module_specification(&mut self, module: &mut ModuleSpecification) {
        self.annotations(&mut module.annotations);
        for value in module.types.values_mut() {
            self.type_specification(&mut value.value);
        }
        for value in module.values.values_mut() {
            self.value_specification(&mut value.value);
        }
    }

    fn module_definition(&mut self, module: &mut ModuleDefinition) {
        for value in module.types.values() {
            self.type_definition(&value.value.value);
        }
        for value in module.values.values() {
            self.visit_definition(&mut IrCursor::root(), &value.value.value);
        }
    }

    fn package_specification(&mut self, package: &mut PackageSpecification) {
        for module in package.modules.values_mut() {
            self.module_specification(module);
        }
    }

    fn package_definition(&mut self, package: &mut PackageDefinition) {
        for module in package.modules.values_mut() {
            self.module_definition(&mut module.value);
        }
    }

    fn distribution(&mut self, distribution: &mut Distribution) {
        match distribution {
            Distribution::Library(content) => {
                self.package_definition(&mut content.def);
                for dependency in content.dependencies.values_mut() {
                    self.package_specification(dependency);
                }
            }
            Distribution::Specs(content) => {
                self.package_specification(&mut content.spec);
                for dependency in content.dependencies.values_mut() {
                    self.package_specification(dependency);
                }
            }
            Distribution::Application(content) => {
                self.package_definition(&mut content.def);
                for dependency in content.dependencies.values_mut() {
                    self.package_definition(dependency);
                }
            }
        }
    }
}

impl V4Visitor for Validator {
    fn visit_type(&mut self, cursor: &mut IrCursor, value: &Type) {
        if self.scope(&value.attributes().metadata).is_some() {
            walk_type(self, cursor, value);
        }
    }

    fn visit_value(&mut self, cursor: &mut IrCursor, value: &Value) {
        if self.scope(&value.attributes().metadata).is_none() {
            return;
        }
        if let Some(inferred) = &value.attributes().inferred_type {
            self.visit_type(cursor, inferred);
        }
        if let Value::Hole(_, _, Some(expected)) = value {
            self.visit_type(cursor, expected);
        }
        walk_value(self, cursor, value);
    }
}
