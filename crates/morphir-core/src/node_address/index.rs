//! Semantic node index for normalized V3 and V4 distributions.

use super::{
    ArtifactRevision, ArtifactSelector, IrFormatVersion, NodeFingerprintBuilder, NodeOwner,
    NodeRoot, NodeStep, NodeUri, Sha256Digest, semantic_json,
};
use crate::format_version::{NormalizedFormatVersion, ScalarValue, SupportTable};
use crate::ir::{classic, v4};
use crate::naming::{Name, PackageName, Path};
use serde::Serialize;
use std::collections::HashMap;

/// The kind of node found at a semantic address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexedNodeKind {
    Distribution,
    Package,
    Module,
    TypeDefinition,
    ValueDefinition,
    Constructor,
    TypeExpression,
    ValueExpression,
    Pattern,
    EntryPoint,
}

/// Distinct outcomes of semantic node lookup or index construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NodeResolutionError {
    #[error("artifact selector does not match the indexed distribution")]
    ArtifactMismatch,
    #[error("artifact selector matches multiple current or pinned distributions")]
    AmbiguousArtifact,
    #[error("IR format version does not match the indexed distribution")]
    FormatVersionMismatch,
    #[error("the requested immutable revision is unavailable in this index")]
    RevisionUnavailable,
    #[error("acquired snapshot bytes do not match the requested revision")]
    RevisionMismatch,
    #[error("the node path or positional guard is stale")]
    StaleTarget,
    #[error("more than one semantic node has the same address")]
    AmbiguousTarget,
    #[error("invalid normalized IR name: {0}")]
    InvalidName(String),
    #[error("cannot fingerprint a semantic child: {0}")]
    InvalidFingerprint(String),
    #[error("cannot decode immutable snapshot: {0}")]
    InvalidSnapshot(String),
}

/// A resolved occurrence in the normalized semantic model. Equal subtrees can
/// have different addresses, so reverse lookup uses the occurrence's URI.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNode {
    pub kind: IndexedNodeKind,
    pub semantic_value: serde_json::Value,
}

#[derive(Debug, Clone)]
struct IndexedNode {
    address: NodeUri,
    node: ResolvedNode,
}

/// An index of addressable nodes in one loaded distribution.
#[derive(Debug)]
pub struct NodeIndex {
    artifact: ArtifactSelector,
    format: IrFormatVersion,
    nodes: HashMap<(NodeRoot, Vec<NodeStep>), IndexedNode>,
}

/// Caller-supplied current distributions and exact acquired snapshots.
///
/// This catalog decodes each snapshot from the exact bytes it hashes, then
/// builds its index. A caller cannot pair one artifact's digest with another's
/// index. Pinned resolution never falls back to a current distribution.
#[derive(Default)]
pub struct NodeCatalog {
    current: Vec<NodeIndex>,
    snapshots: Vec<(Sha256Digest, NodeIndex)>,
}

impl NodeCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_current(&mut self, index: NodeIndex) {
        self.current.push(index);
    }

    /// Register one V4 JSON snapshot, optionally checking an expected pin.
    pub fn add_v4_json_snapshot(
        &mut self,
        bytes: &[u8],
        expected: Option<&Sha256Digest>,
    ) -> Result<Sha256Digest, NodeResolutionError> {
        let digest = verify_snapshot_digest(bytes, expected)?;
        let text = std::str::from_utf8(bytes)
            .map_err(|error| NodeResolutionError::InvalidSnapshot(error.to_string()))?;
        let (file, _) = crate::ir::json::read_ir_file(text)
            .map_err(|error| NodeResolutionError::InvalidSnapshot(error.to_string()))?;
        let index = NodeIndex::v4_file(&file)?;
        self.snapshots.push((digest.clone(), index));
        Ok(digest)
    }

    /// Register one Classic V3 JSON snapshot, optionally checking a pin.
    pub fn add_v3_json_snapshot(
        &mut self,
        bytes: &[u8],
        expected: Option<&Sha256Digest>,
    ) -> Result<Sha256Digest, NodeResolutionError> {
        let digest = verify_snapshot_digest(bytes, expected)?;
        let index = NodeIndex::v3_json(bytes)?;
        self.snapshots.push((digest.clone(), index));
        Ok(digest)
    }

    pub fn resolve(&self, address: &NodeUri) -> Result<IndexedNodeKind, NodeResolutionError> {
        self.resolve_node(address).map(|node| node.kind)
    }

    pub fn resolve_node(&self, address: &NodeUri) -> Result<&ResolvedNode, NodeResolutionError> {
        match &address.revision {
            ArtifactRevision::Current => {
                let mut matching = self
                    .current
                    .iter()
                    .filter(|index| index.artifact == address.artifact);
                let index = matching
                    .next()
                    .ok_or(NodeResolutionError::ArtifactMismatch)?;
                if matching.next().is_some() {
                    return Err(NodeResolutionError::AmbiguousArtifact);
                }
                index.resolve_node(address)
            }
            ArtifactRevision::Pinned(digest) => {
                let mut matching = self.snapshots.iter().filter(|(candidate, index)| {
                    candidate == digest && index.artifact == address.artifact
                });
                let (_, index) = matching
                    .next()
                    .ok_or(NodeResolutionError::RevisionUnavailable)?;
                if matching.next().is_some() {
                    return Err(NodeResolutionError::AmbiguousArtifact);
                }
                if address.format != index.format {
                    return Err(NodeResolutionError::FormatVersionMismatch);
                }
                let node = index
                    .nodes
                    .get(&(address.root.clone(), address.steps.clone()))
                    .ok_or(NodeResolutionError::StaleTarget)?;
                if let Some(guard) = &address.guard
                    && node.address.guard.as_ref() != Some(guard)
                {
                    return Err(NodeResolutionError::StaleTarget);
                }
                Ok(&node.node)
            }
        }
    }
}

fn verify_snapshot_digest(
    bytes: &[u8],
    expected: Option<&Sha256Digest>,
) -> Result<Sha256Digest, NodeResolutionError> {
    let actual = Sha256Digest::from_bytes(bytes);
    if expected.is_some_and(|expected| expected != &actual) {
        return Err(NodeResolutionError::RevisionMismatch);
    }
    Ok(actual)
}

#[derive(Clone)]
struct WalkContext {
    root: NodeRoot,
    steps: Vec<NodeStep>,
    lineage: Option<NodeFingerprintBuilder>,
}

impl WalkContext {
    fn root(root: NodeRoot) -> Self {
        Self {
            root,
            steps: Vec::new(),
            lineage: None,
        }
    }

    fn named(&self, step: NodeStep) -> Self {
        let mut next = self.clone();
        next.steps.push(step);
        next
    }

    fn ordered<T: Serialize>(
        &self,
        step: NodeStep,
        child: &T,
    ) -> Result<Self, NodeResolutionError> {
        let mut next = self.named(step.clone());
        let mut lineage = next.lineage.unwrap_or_default();
        lineage
            .push(&step, child)
            .map_err(|error| NodeResolutionError::InvalidFingerprint(error.to_string()))?;
        next.lineage = Some(lineage);
        Ok(next)
    }
}

impl NodeIndex {
    /// Index a complete V4 file using its exact normalized format version.
    pub fn v4_file(file: &v4::IRFile) -> Result<Self, NodeResolutionError> {
        let artifact = ArtifactSelector::Package(file.distribution.package_name().clone());
        Self::v4_file_with_selector(file, artifact)
    }

    /// Index a complete V4 file under a package selector or workspace alias.
    pub fn v4_file_with_selector(
        file: &v4::IRFile,
        artifact: ArtifactSelector,
    ) -> Result<Self, NodeResolutionError> {
        let normalized = file
            .format_version
            .normalize()
            .map_err(|_| NodeResolutionError::FormatVersionMismatch)?;
        if normalized.release.major() != 4 || !normalized.is_supported() {
            return Err(NodeResolutionError::FormatVersionMismatch);
        }
        Self::v4_with_format(&file.distribution, artifact, normalized.release)
    }

    /// Index a normalized V4 distribution and all currently supported semantic nodes.
    pub fn v4(distribution: &v4::Distribution) -> Result<Self, NodeResolutionError> {
        let artifact = ArtifactSelector::Package(distribution.package_name().clone());
        Self::v4_with_selector(distribution, artifact)
    }

    /// Index a V4 distribution under its package name or an exact workspace alias.
    pub fn v4_with_selector(
        distribution: &v4::Distribution,
        artifact: ArtifactSelector,
    ) -> Result<Self, NodeResolutionError> {
        Self::v4_with_format(distribution, artifact, IrFormatVersion::new(4, 0, 0))
    }

    fn v4_with_format(
        distribution: &v4::Distribution,
        artifact: ArtifactSelector,
        format: IrFormatVersion,
    ) -> Result<Self, NodeResolutionError> {
        if let ArtifactSelector::Package(package) = &artifact
            && package != distribution.package_name()
        {
            return Err(NodeResolutionError::ArtifactMismatch);
        }
        let mut index = Self::new(artifact, format);
        index.add(
            &WalkContext::root(NodeRoot::Distribution),
            IndexedNodeKind::Distribution,
            distribution,
        )?;
        match distribution {
            v4::Distribution::Library(library) => {
                index.add(
                    &WalkContext::root(NodeRoot::Package),
                    IndexedNodeKind::Package,
                    &library.def,
                )?;
                index.v4_definition_package(NodeOwner::OwnPackage, &library.def)?;
                for (package, spec) in &library.dependencies {
                    let package = parse_package(package)?;
                    index.add(
                        &WalkContext::root(NodeRoot::Dependency(package.clone())),
                        IndexedNodeKind::Package,
                        spec,
                    )?;
                    index.v4_specification_package(NodeOwner::Dependency(package), spec)?;
                }
            }
            v4::Distribution::Specs(specs) => {
                index.add(
                    &WalkContext::root(NodeRoot::Package),
                    IndexedNodeKind::Package,
                    &specs.spec,
                )?;
                index.v4_specification_package(NodeOwner::OwnPackage, &specs.spec)?;
                for (package, spec) in &specs.dependencies {
                    let package = parse_package(package)?;
                    index.add(
                        &WalkContext::root(NodeRoot::Dependency(package.clone())),
                        IndexedNodeKind::Package,
                        spec,
                    )?;
                    index.v4_specification_package(NodeOwner::Dependency(package), spec)?;
                }
            }
            v4::Distribution::Application(application) => {
                index.add(
                    &WalkContext::root(NodeRoot::Package),
                    IndexedNodeKind::Package,
                    &application.def,
                )?;
                index.v4_definition_package(NodeOwner::OwnPackage, &application.def)?;
                for (package, definition) in &application.dependencies {
                    let package = parse_package(package)?;
                    index.add(
                        &WalkContext::root(NodeRoot::Dependency(package.clone())),
                        IndexedNodeKind::Package,
                        definition,
                    )?;
                    index.v4_definition_package(NodeOwner::Dependency(package), definition)?;
                }
                for (key, entry_point) in &application.entry_points {
                    index.add(
                        &WalkContext::root(NodeRoot::EntryPoint(key.clone())),
                        IndexedNodeKind::EntryPoint,
                        entry_point,
                    )?;
                }
            }
        }
        Ok(index)
    }

    /// Index one Classic V3 library under an explicit artifact selector.
    pub fn v3(
        distribution: &classic::Distribution,
        artifact: ArtifactSelector,
    ) -> Result<Self, NodeResolutionError> {
        let format = match &distribution.distribution {
            classic::DistributionBody::Library(..) => IrFormatVersion::new(3, 0, 0),
            classic::DistributionBody::Specs(..) => IrFormatVersion::new(3, 1, 0),
        };
        Self::v3_with_format(distribution, artifact, format)
    }

    /// Index exact Classic V3 JSON, preserving its declared release.
    pub fn v3_json(bytes: &[u8]) -> Result<Self, NodeResolutionError> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| NodeResolutionError::InvalidSnapshot(error.to_string()))?;
        let declared = value
            .get("formatVersion")
            .ok_or_else(|| NodeResolutionError::InvalidSnapshot("missing formatVersion".into()))?;
        let scalar = ScalarValue::from_json(declared)
            .map_err(|error| NodeResolutionError::InvalidSnapshot(error.to_string()))?;
        let normalized = NormalizedFormatVersion::from_scalar(&scalar, &SupportTable::reference())
            .map_err(|error| NodeResolutionError::InvalidSnapshot(error.to_string()))?;
        if !normalized.is_supported() {
            return Err(NodeResolutionError::FormatVersionMismatch);
        }
        let distribution: classic::Distribution = serde_json::from_value(value)
            .map_err(|error| NodeResolutionError::InvalidSnapshot(error.to_string()))?;
        let package = match &distribution.distribution {
            classic::DistributionBody::Library(package, _, _)
            | classic::DistributionBody::Specs(package, _, _) => package,
        };
        let selector = ArtifactSelector::Package(PackageName::new(classic_path(package)?));
        Self::v3_with_format(&distribution, selector, normalized.release)
    }

    fn v3_with_format(
        distribution: &classic::Distribution,
        artifact: ArtifactSelector,
        format: IrFormatVersion,
    ) -> Result<Self, NodeResolutionError> {
        if distribution.format_version != 3 {
            return Err(NodeResolutionError::FormatVersionMismatch);
        }
        if format.major() != 3
            || matches!(
                distribution.distribution,
                classic::DistributionBody::Specs(..)
            ) && format < IrFormatVersion::new(3, 1, 0)
        {
            return Err(NodeResolutionError::FormatVersionMismatch);
        }
        let mut index = Self::new(artifact, format);
        index.add(
            &WalkContext::root(NodeRoot::Distribution),
            IndexedNodeKind::Distribution,
            distribution,
        )?;
        let (package_path, dependencies) = match &distribution.distribution {
            classic::DistributionBody::Library(path, dependencies, _)
            | classic::DistributionBody::Specs(path, dependencies, _) => (path, dependencies),
        };
        if let ArtifactSelector::Package(selected) = &index.artifact
            && selected.as_path() != &classic_path(package_path)?
        {
            return Err(NodeResolutionError::ArtifactMismatch);
        }
        match &distribution.distribution {
            classic::DistributionBody::Library(_, _, package) => {
                index.add(
                    &WalkContext::root(NodeRoot::Package),
                    IndexedNodeKind::Package,
                    package,
                )?;
                index.v3_definition_package(NodeOwner::OwnPackage, package)?;
            }
            classic::DistributionBody::Specs(_, _, package) => {
                index.add(
                    &WalkContext::root(NodeRoot::Package),
                    IndexedNodeKind::Package,
                    package,
                )?;
                index.v3_specification_package(NodeOwner::OwnPackage, package)?;
            }
        }
        for (path, specification) in dependencies {
            let package = PackageName::new(classic_path(path)?);
            index.add(
                &WalkContext::root(NodeRoot::Dependency(package.clone())),
                IndexedNodeKind::Package,
                specification,
            )?;
            index.v3_specification_package(NodeOwner::Dependency(package), specification)?;
        }
        Ok(index)
    }

    fn new(artifact: ArtifactSelector, format: IrFormatVersion) -> Self {
        Self {
            artifact,
            format,
            nodes: HashMap::new(),
        }
    }

    fn add<T: Serialize>(
        &mut self,
        context: &WalkContext,
        kind: IndexedNodeKind,
        semantic_node: &T,
    ) -> Result<(), NodeResolutionError> {
        let address = NodeUri::new(
            self.artifact.clone(),
            self.format,
            context.root.clone(),
            context.steps.clone(),
            ArtifactRevision::Current,
            context.lineage.clone().map(NodeFingerprintBuilder::finish),
        )
        .map_err(|error| NodeResolutionError::InvalidName(error.to_string()))?;
        let key = (context.root.clone(), context.steps.clone());
        let semantic_value = semantic_json(semantic_node)
            .map_err(|error| NodeResolutionError::InvalidFingerprint(error.to_string()))?;
        if self
            .nodes
            .insert(
                key,
                IndexedNode {
                    address,
                    node: ResolvedNode {
                        kind,
                        semantic_value,
                    },
                },
            )
            .is_some()
        {
            return Err(NodeResolutionError::AmbiguousTarget);
        }
        Ok(())
    }

    /// Resolve one address in this loaded distribution. Pinned addresses require
    /// an immutable snapshot resolver and are not silently applied to current.
    pub fn resolve(&self, address: &NodeUri) -> Result<IndexedNodeKind, NodeResolutionError> {
        self.resolve_node(address).map(|node| node.kind)
    }

    pub fn resolve_node(&self, address: &NodeUri) -> Result<&ResolvedNode, NodeResolutionError> {
        if address.artifact != self.artifact {
            return Err(NodeResolutionError::ArtifactMismatch);
        }
        if address.format != self.format {
            return Err(NodeResolutionError::FormatVersionMismatch);
        }
        if !matches!(address.revision, ArtifactRevision::Current) {
            return Err(NodeResolutionError::RevisionUnavailable);
        }
        let node = self
            .nodes
            .get(&(address.root.clone(), address.steps.clone()))
            .ok_or(NodeResolutionError::StaleTarget)?;
        if node.address.guard != address.guard {
            return Err(NodeResolutionError::StaleTarget);
        }
        Ok(&node.node)
    }

    /// Enumerate canonical current addresses in this distribution.
    pub fn addresses(&self) -> impl Iterator<Item = &NodeUri> {
        self.nodes.values().map(|node| &node.address)
    }

    /// Enumerate occurrences with their canonical addresses, including equal
    /// semantic subtrees at distinct positions.
    pub fn nodes(&self) -> impl Iterator<Item = (&NodeUri, &ResolvedNode)> {
        self.nodes.values().map(|node| (&node.address, &node.node))
    }

    /// Return the canonical current URI for a known typed path, including a
    /// computed guard when that path enters ordered children.
    pub fn address_for(
        &self,
        root: &NodeRoot,
        steps: &[NodeStep],
    ) -> Result<NodeUri, NodeResolutionError> {
        self.nodes
            .get(&(root.clone(), steps.to_vec()))
            .map(|node| node.address.clone())
            .ok_or(NodeResolutionError::StaleTarget)
    }

    fn v4_definition_package(
        &mut self,
        owner: NodeOwner,
        package: &v4::PackageDefinition,
    ) -> Result<(), NodeResolutionError> {
        for (module_name, controlled) in &package.modules {
            let module = parse_path(module_name)?;
            let definition = &controlled.value;
            self.add(
                &WalkContext::root(NodeRoot::Module {
                    owner: owner.clone(),
                    module: module.clone(),
                }),
                IndexedNodeKind::Module,
                definition,
            )?;
            for (name, controlled) in &definition.types {
                let context = WalkContext::root(NodeRoot::Type {
                    owner: owner.clone(),
                    module: module.clone(),
                    name: parse_name(name)?,
                });
                self.add(
                    &context,
                    IndexedNodeKind::TypeDefinition,
                    &controlled.value.value,
                )?;
                self.v4_type_definition(&context, &controlled.value.value)?;
            }
            for (name, controlled) in &definition.values {
                let context = WalkContext::root(NodeRoot::Value {
                    owner: owner.clone(),
                    module: module.clone(),
                    name: parse_name(name)?,
                });
                self.add(
                    &context,
                    IndexedNodeKind::ValueDefinition,
                    &controlled.value.value,
                )?;
                self.v4_value_definition(&context, &controlled.value.value)?;
            }
        }
        Ok(())
    }

    fn v4_specification_package(
        &mut self,
        owner: NodeOwner,
        package: &v4::PackageSpecification,
    ) -> Result<(), NodeResolutionError> {
        for (module_name, specification) in &package.modules {
            let module = parse_path(module_name)?;
            self.add(
                &WalkContext::root(NodeRoot::Module {
                    owner: owner.clone(),
                    module: module.clone(),
                }),
                IndexedNodeKind::Module,
                specification,
            )?;
            for (name, documented) in &specification.types {
                let context = WalkContext::root(NodeRoot::Type {
                    owner: owner.clone(),
                    module: module.clone(),
                    name: parse_name(name)?,
                });
                self.add(&context, IndexedNodeKind::TypeDefinition, &documented.value)?;
                self.v4_type_specification(&context, &documented.value)?;
            }
            for (name, documented) in &specification.values {
                let context = WalkContext::root(NodeRoot::Value {
                    owner: owner.clone(),
                    module: module.clone(),
                    name: parse_name(name)?,
                });
                self.add(
                    &context,
                    IndexedNodeKind::ValueDefinition,
                    &documented.value,
                )?;
                self.v4_value_specification(&context, &documented.value)?;
            }
        }
        Ok(())
    }

    fn v4_type_definition(
        &mut self,
        context: &WalkContext,
        definition: &v4::TypeDefinition,
    ) -> Result<(), NodeResolutionError> {
        match definition {
            v4::TypeDefinition::TypeAliasDefinition { type_expr, .. } => {
                self.v4_type(&context.named(NodeStep::TypeExpression), type_expr)?
            }
            v4::TypeDefinition::CustomTypeDefinition { constructors, .. } => {
                for constructor in &constructors.value {
                    let child =
                        context.named(NodeStep::CustomConstructor(constructor.name.clone()));
                    self.add(&child, IndexedNodeKind::Constructor, constructor)?;
                    for (index, argument) in constructor.args.iter().enumerate() {
                        let argument_context = child
                            .ordered(NodeStep::ConstructorArgument(index), &argument.arg_type)?;
                        self.v4_type(&argument_context, &argument.arg_type)?;
                    }
                }
            }
            v4::TypeDefinition::IncompleteTypeDefinition {
                incompleteness,
                partial_type_expr,
                ..
            } => {
                if let Some(ty) = partial_type_expr {
                    self.v4_type(&context.named(NodeStep::PartialTypeExpression), ty)?;
                }
                if let v4::Incompleteness::Hole {
                    partial_body: Some(ty),
                    ..
                } = incompleteness
                {
                    self.v4_type(&context.named(NodeStep::HoleExpectedType), ty)?;
                }
            }
        }
        Ok(())
    }

    fn v4_type_specification(
        &mut self,
        context: &WalkContext,
        specification: &v4::TypeSpecification,
    ) -> Result<(), NodeResolutionError> {
        match specification {
            v4::TypeSpecification::TypeAliasSpecification { type_expr, .. } => {
                self.v4_type(&context.named(NodeStep::TypeExpression), type_expr)?
            }
            v4::TypeSpecification::CustomTypeSpecification { constructors, .. } => {
                for constructor in constructors {
                    let child =
                        context.named(NodeStep::CustomConstructor(constructor.name.clone()));
                    self.add(&child, IndexedNodeKind::Constructor, constructor)?;
                    for (index, argument) in constructor.args.iter().enumerate() {
                        let argument_context = child
                            .ordered(NodeStep::ConstructorArgument(index), &argument.arg_type)?;
                        self.v4_type(&argument_context, &argument.arg_type)?;
                    }
                }
            }
            v4::TypeSpecification::DerivedTypeSpecification { base_type, .. } => {
                self.v4_type(&context.named(NodeStep::DerivedBaseType), base_type)?
            }
            v4::TypeSpecification::OpaqueTypeSpecification { .. } => {}
        }
        Ok(())
    }

    fn v4_type(&mut self, context: &WalkContext, ty: &v4::Type) -> Result<(), NodeResolutionError> {
        self.add(context, IndexedNodeKind::TypeExpression, ty)?;
        match ty {
            v4::Type::Record(_, fields) => {
                for field in fields {
                    self.v4_type(
                        &context.named(NodeStep::RecordField(field.name.clone())),
                        &field.tpe,
                    )?;
                }
            }
            v4::Type::ExtensibleRecord(_, _, fields) => {
                for field in fields {
                    self.v4_type(
                        &context.named(NodeStep::ExtensibleRecordField(field.name.clone())),
                        &field.tpe,
                    )?;
                }
            }
            v4::Type::Function(_, parameter, result) => {
                self.v4_type(&context.named(NodeStep::TypeFunctionParameter), parameter)?;
                self.v4_type(&context.named(NodeStep::TypeFunctionResult), result)?;
            }
            v4::Type::Reference(_, _, arguments) => {
                for (index, argument) in arguments.iter().enumerate() {
                    let child = context.ordered(NodeStep::ReferenceArgument(index), argument)?;
                    self.v4_type(&child, argument)?;
                }
            }
            v4::Type::Tuple(_, elements) => {
                for (index, element) in elements.iter().enumerate() {
                    let child = context.ordered(NodeStep::TupleElement(index), element)?;
                    self.v4_type(&child, element)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn v4_value_definition(
        &mut self,
        context: &WalkContext,
        definition: &v4::ValueDefinition,
    ) -> Result<(), NodeResolutionError> {
        for (name, ty) in &definition.input_types {
            self.v4_type(
                &context.named(NodeStep::ValueInputType(parse_name(name)?)),
                ty,
            )?;
        }
        if let Some(output) = &definition.output_type {
            self.v4_type(&context.named(NodeStep::ValueOutputType), output)?;
        }
        match &definition.body {
            v4::ValueBody::Expression(body) => {
                self.v4_value(&context.named(NodeStep::Body), body)?
            }
            v4::ValueBody::External {
                fallback: Some(body),
                ..
            } => self.v4_value(&context.named(NodeStep::ExternalFallback), body)?,
            v4::ValueBody::Incomplete {
                partial_body,
                incompleteness,
            } => {
                if let Some(body) = partial_body {
                    self.v4_value(&context.named(NodeStep::IncompletePartialBody), body)?;
                }
                if let v4::Incompleteness::Hole {
                    partial_body: Some(ty),
                    ..
                } = incompleteness
                {
                    self.v4_type(&context.named(NodeStep::HoleExpectedType), ty)?;
                }
            }
            v4::ValueBody::Native { .. } | v4::ValueBody::External { fallback: None, .. } => {}
        }
        Ok(())
    }

    fn v4_value_specification(
        &mut self,
        context: &WalkContext,
        specification: &v4::ValueSpecification,
    ) -> Result<(), NodeResolutionError> {
        for (name, ty) in &specification.inputs {
            self.v4_type(
                &context.named(NodeStep::ValueInputType(parse_name(name)?)),
                ty,
            )?;
        }
        self.v4_type(
            &context.named(NodeStep::ValueOutputType),
            &specification.output,
        )
    }

    fn v4_value(
        &mut self,
        context: &WalkContext,
        value: &v4::Value,
    ) -> Result<(), NodeResolutionError> {
        self.add(context, IndexedNodeKind::ValueExpression, value)?;
        match value {
            v4::Value::Apply(_, function, argument) => {
                self.v4_value(&context.named(NodeStep::ApplyFunction), function)?;
                self.v4_value(&context.named(NodeStep::ApplyArgument), argument)?;
            }
            v4::Value::Field(_, subject, _) => {
                self.v4_value(&context.named(NodeStep::FieldSubject), subject)?
            }
            v4::Value::Destructure(_, pattern, subject, body) => {
                self.v4_pattern(&context.named(NodeStep::DestructurePattern), pattern)?;
                self.v4_value(&context.named(NodeStep::DestructureValue), subject)?;
                self.v4_value(&context.named(NodeStep::DestructureBody), body)?;
            }
            v4::Value::IfThenElse(_, condition, then_value, else_value) => {
                self.v4_value(&context.named(NodeStep::IfCondition), condition)?;
                self.v4_value(&context.named(NodeStep::IfThen), then_value)?;
                self.v4_value(&context.named(NodeStep::IfElse), else_value)?;
            }
            v4::Value::Lambda(_, pattern, body) => {
                self.v4_pattern(&context.named(NodeStep::LambdaPattern), pattern)?;
                self.v4_value(&context.named(NodeStep::LambdaBody), body)?;
            }
            v4::Value::LetDefinition(_, name, definition, body) => {
                let child = context.named(NodeStep::LetDefinition(name.clone()));
                self.add(
                    &child,
                    IndexedNodeKind::ValueDefinition,
                    definition.as_ref(),
                )?;
                self.v4_value_definition(&child, definition)?;
                self.v4_value(&context.named(NodeStep::LetBody), body)?;
            }
            v4::Value::LetRecursion(_, definitions, body) => {
                for binding in definitions {
                    let child = context.named(NodeStep::LetDefinition(binding.name().clone()));
                    self.add(
                        &child,
                        IndexedNodeKind::ValueDefinition,
                        binding.definition(),
                    )?;
                    self.v4_value_definition(&child, binding.definition())?;
                }
                self.v4_value(&context.named(NodeStep::LetBody), body)?;
            }
            v4::Value::List(_, elements) => {
                for (index, element) in elements.iter().enumerate() {
                    let child = context.ordered(NodeStep::ListElement(index), element)?;
                    self.v4_value(&child, element)?;
                }
            }
            v4::Value::Record(_, fields) => {
                for field in fields {
                    self.v4_value(
                        &context.named(NodeStep::RecordField(field.name().clone())),
                        field.value(),
                    )?;
                }
            }
            v4::Value::Tuple(_, elements) => {
                for (index, element) in elements.iter().enumerate() {
                    let child = context.ordered(NodeStep::TupleElement(index), element)?;
                    self.v4_value(&child, element)?;
                }
            }
            v4::Value::PatternMatch(_, subject, cases) => {
                self.v4_value(&context.named(NodeStep::PatternMatchSubject), subject)?;
                for (index, case) in cases.iter().enumerate() {
                    let case_content = (case.pattern(), case.body());
                    let pattern =
                        context.ordered(NodeStep::PatternMatchCasePattern(index), &case_content)?;
                    self.v4_pattern(&pattern, case.pattern())?;
                    let body =
                        context.ordered(NodeStep::PatternMatchCaseBody(index), &case_content)?;
                    self.v4_value(&body, case.body())?;
                }
            }
            v4::Value::UpdateRecord(_, subject, fields) => {
                self.v4_value(&context.named(NodeStep::UpdateSubject), subject)?;
                for field in fields {
                    self.v4_value(
                        &context.named(NodeStep::UpdateField(field.name().clone())),
                        field.value(),
                    )?;
                }
            }
            v4::Value::Hole(_, _, Some(expected)) => {
                self.v4_type(&context.named(NodeStep::HoleExpectedType), expected)?
            }
            v4::Value::Literal(_, _)
            | v4::Value::Constructor(_, _)
            | v4::Value::Variable(_, _)
            | v4::Value::Reference(_, _)
            | v4::Value::FieldFunction(_, _)
            | v4::Value::Unit(_)
            | v4::Value::Hole(_, _, None) => {}
        }
        Ok(())
    }

    fn v4_pattern(
        &mut self,
        context: &WalkContext,
        pattern: &v4::Pattern,
    ) -> Result<(), NodeResolutionError> {
        self.add(context, IndexedNodeKind::Pattern, pattern)?;
        match pattern {
            v4::Pattern::AsPattern(_, child, _) => {
                self.v4_pattern(&context.named(NodeStep::AsPatternChild), child)?
            }
            v4::Pattern::TuplePattern(_, children) => {
                for (index, child) in children.iter().enumerate() {
                    let path = context.ordered(NodeStep::PatternTupleElement(index), child)?;
                    self.v4_pattern(&path, child)?;
                }
            }
            v4::Pattern::ConstructorPattern(_, _, children) => {
                for (index, child) in children.iter().enumerate() {
                    let path =
                        context.ordered(NodeStep::PatternConstructorArgument(index), child)?;
                    self.v4_pattern(&path, child)?;
                }
            }
            v4::Pattern::HeadTailPattern(_, head, tail) => {
                self.v4_pattern(&context.named(NodeStep::HeadTailHead), head)?;
                self.v4_pattern(&context.named(NodeStep::HeadTailTail), tail)?;
            }
            v4::Pattern::WildcardPattern(_)
            | v4::Pattern::EmptyListPattern(_)
            | v4::Pattern::LiteralPattern(_, _)
            | v4::Pattern::UnitPattern(_) => {}
        }
        Ok(())
    }

    fn v3_definition_package(
        &mut self,
        owner: NodeOwner,
        package: &classic::PackageDefinition<classic::Attrs, classic::Type<classic::Attrs>>,
    ) -> Result<(), NodeResolutionError> {
        for entry in &package.modules {
            let module = classic_path(&entry.path)?;
            self.add(
                &WalkContext::root(NodeRoot::Module {
                    owner: owner.clone(),
                    module: module.clone(),
                }),
                IndexedNodeKind::Module,
                &entry.definition.value,
            )?;
            for (name, controlled) in &entry.definition.value.types {
                let context = WalkContext::root(NodeRoot::Type {
                    owner: owner.clone(),
                    module: module.clone(),
                    name: classic_name(name)?,
                });
                self.add(
                    &context,
                    IndexedNodeKind::TypeDefinition,
                    &controlled.value.value,
                )?;
                self.v3_type_definition(&context, &controlled.value.value)?;
            }
            for (name, controlled) in &entry.definition.value.values {
                let context = WalkContext::root(NodeRoot::Value {
                    owner: owner.clone(),
                    module: module.clone(),
                    name: classic_name(name)?,
                });
                self.add(
                    &context,
                    IndexedNodeKind::ValueDefinition,
                    &controlled.value.value,
                )?;
                self.v3_value_definition(&context, &controlled.value.value)?;
            }
        }
        Ok(())
    }

    fn v3_specification_package(
        &mut self,
        owner: NodeOwner,
        package: &classic::PackageSpecification<classic::Attrs>,
    ) -> Result<(), NodeResolutionError> {
        for entry in &package.modules {
            let module = classic_path(&entry.path)?;
            self.add(
                &WalkContext::root(NodeRoot::Module {
                    owner: owner.clone(),
                    module: module.clone(),
                }),
                IndexedNodeKind::Module,
                &entry.specification,
            )?;
            for (name, documented) in &entry.specification.types {
                let context = WalkContext::root(NodeRoot::Type {
                    owner: owner.clone(),
                    module: module.clone(),
                    name: classic_name(name)?,
                });
                self.add(&context, IndexedNodeKind::TypeDefinition, &documented.value)?;
                self.v3_type_specification(&context, &documented.value)?;
            }
            for (name, documented) in &entry.specification.values {
                let context = WalkContext::root(NodeRoot::Value {
                    owner: owner.clone(),
                    module: module.clone(),
                    name: classic_name(name)?,
                });
                self.add(
                    &context,
                    IndexedNodeKind::ValueDefinition,
                    &documented.value,
                )?;
                for input in &documented.value.inputs {
                    self.v3_type(
                        &context.named(NodeStep::ValueInputType(classic_name(&input.name)?)),
                        &input.ty,
                    )?;
                }
                self.v3_type(
                    &context.named(NodeStep::ValueOutputType),
                    &documented.value.output,
                )?;
            }
        }
        Ok(())
    }

    fn v3_type_definition(
        &mut self,
        context: &WalkContext,
        definition: &classic::TypeDefinition<classic::Attrs>,
    ) -> Result<(), NodeResolutionError> {
        match definition {
            classic::TypeDefinition::Alias(_, ty) => {
                self.v3_type(&context.named(NodeStep::TypeExpression), ty)?
            }
            classic::TypeDefinition::Custom(_, controlled) => {
                self.v3_constructors(context, &controlled.value)?
            }
        }
        Ok(())
    }

    fn v3_type_specification(
        &mut self,
        context: &WalkContext,
        specification: &classic::TypeSpecification<classic::Attrs>,
    ) -> Result<(), NodeResolutionError> {
        match specification {
            classic::TypeSpecification::Alias(_, ty) => {
                self.v3_type(&context.named(NodeStep::TypeExpression), ty)?
            }
            classic::TypeSpecification::Custom(_, constructors) => {
                self.v3_constructors(context, constructors)?
            }
            classic::TypeSpecification::Derived(_, configuration) => self.v3_type(
                &context.named(NodeStep::DerivedBaseType),
                &configuration.base_type,
            )?,
            classic::TypeSpecification::Opaque(_) => {}
        }
        Ok(())
    }

    fn v3_constructors(
        &mut self,
        context: &WalkContext,
        constructors: &[classic::Constructor<classic::Attrs>],
    ) -> Result<(), NodeResolutionError> {
        for constructor in constructors {
            let child = context.named(NodeStep::CustomConstructor(classic_name(
                &constructor.name,
            )?));
            self.add(&child, IndexedNodeKind::Constructor, constructor)?;
            for (index, (_, ty)) in constructor.args.iter().enumerate() {
                let argument = child.ordered(NodeStep::ConstructorArgument(index), ty)?;
                self.v3_type(&argument, ty)?;
            }
        }
        Ok(())
    }

    fn v3_value_definition(
        &mut self,
        context: &WalkContext,
        definition: &classic::ValueDefinition<classic::Attrs, classic::Type<classic::Attrs>>,
    ) -> Result<(), NodeResolutionError> {
        for input in &definition.input_types {
            self.v3_type(
                &context.named(NodeStep::ValueInputType(classic_name(&input.name)?)),
                &input.ty,
            )?;
            self.v3_type(
                &context.named(NodeStep::ValueInputAnnotation(classic_name(&input.name)?)),
                &input.annotation,
            )?;
        }
        self.v3_type(
            &context.named(NodeStep::ValueOutputType),
            &definition.output_type,
        )?;
        self.v3_value(&context.named(NodeStep::Body), &definition.body)
    }

    fn v3_type(
        &mut self,
        context: &WalkContext,
        ty: &classic::Type<classic::Attrs>,
    ) -> Result<(), NodeResolutionError> {
        self.add(context, IndexedNodeKind::TypeExpression, ty)?;
        match ty {
            classic::Type::Record(_, fields) => {
                for field in fields {
                    self.v3_type(
                        &context.named(NodeStep::RecordField(classic_name(&field.name)?)),
                        &field.ty,
                    )?;
                }
            }
            classic::Type::ExtensibleRecord(_, _, fields) => {
                for field in fields {
                    self.v3_type(
                        &context.named(NodeStep::ExtensibleRecordField(classic_name(&field.name)?)),
                        &field.ty,
                    )?;
                }
            }
            classic::Type::Function(_, parameter, result) => {
                self.v3_type(&context.named(NodeStep::TypeFunctionParameter), parameter)?;
                self.v3_type(&context.named(NodeStep::TypeFunctionResult), result)?;
            }
            classic::Type::Reference(_, _, arguments) => {
                for (index, argument) in arguments.iter().enumerate() {
                    let child = context.ordered(NodeStep::ReferenceArgument(index), argument)?;
                    self.v3_type(&child, argument)?;
                }
            }
            classic::Type::Tuple(_, elements) => {
                for (index, element) in elements.iter().enumerate() {
                    let child = context.ordered(NodeStep::TupleElement(index), element)?;
                    self.v3_type(&child, element)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn v3_value(
        &mut self,
        context: &WalkContext,
        value: &classic::Value<classic::Attrs, classic::Type<classic::Attrs>>,
    ) -> Result<(), NodeResolutionError> {
        self.add(context, IndexedNodeKind::ValueExpression, value)?;
        match value {
            classic::Value::Apply(_, function, argument) => {
                self.v3_value(&context.named(NodeStep::ApplyFunction), function)?;
                self.v3_value(&context.named(NodeStep::ApplyArgument), argument)?;
            }
            classic::Value::Field(_, subject, _) => {
                self.v3_value(&context.named(NodeStep::FieldSubject), subject)?
            }
            classic::Value::Destructure(_, pattern, subject, body) => {
                self.v3_pattern(&context.named(NodeStep::DestructurePattern), pattern)?;
                self.v3_value(&context.named(NodeStep::DestructureValue), subject)?;
                self.v3_value(&context.named(NodeStep::DestructureBody), body)?;
            }
            classic::Value::IfThenElse(_, condition, then_value, else_value) => {
                self.v3_value(&context.named(NodeStep::IfCondition), condition)?;
                self.v3_value(&context.named(NodeStep::IfThen), then_value)?;
                self.v3_value(&context.named(NodeStep::IfElse), else_value)?;
            }
            classic::Value::Lambda(_, pattern, body) => {
                self.v3_pattern(&context.named(NodeStep::LambdaPattern), pattern)?;
                self.v3_value(&context.named(NodeStep::LambdaBody), body)?;
            }
            classic::Value::LetDefinition(_, name, definition, body) => {
                let child = context.named(NodeStep::LetDefinition(classic_name(name)?));
                self.add(
                    &child,
                    IndexedNodeKind::ValueDefinition,
                    definition.as_ref(),
                )?;
                self.v3_value_definition(&child, definition)?;
                self.v3_value(&context.named(NodeStep::LetBody), body)?;
            }
            classic::Value::LetRecursion(_, definitions, body) => {
                for (name, definition) in definitions {
                    let child = context.named(NodeStep::LetDefinition(classic_name(name)?));
                    self.add(&child, IndexedNodeKind::ValueDefinition, definition)?;
                    self.v3_value_definition(&child, definition)?;
                }
                self.v3_value(&context.named(NodeStep::LetBody), body)?;
            }
            classic::Value::List(_, elements) => {
                for (index, element) in elements.iter().enumerate() {
                    let child = context.ordered(NodeStep::ListElement(index), element)?;
                    self.v3_value(&child, element)?;
                }
            }
            classic::Value::Record(_, fields) => {
                for (name, value) in fields {
                    self.v3_value(
                        &context.named(NodeStep::RecordField(classic_name(name)?)),
                        value,
                    )?;
                }
            }
            classic::Value::Tuple(_, elements) => {
                for (index, element) in elements.iter().enumerate() {
                    let child = context.ordered(NodeStep::TupleElement(index), element)?;
                    self.v3_value(&child, element)?;
                }
            }
            classic::Value::PatternMatch(_, subject, cases) => {
                self.v3_value(&context.named(NodeStep::PatternMatchSubject), subject)?;
                for (index, (pattern, body)) in cases.iter().enumerate() {
                    let pattern_context = context
                        .ordered(NodeStep::PatternMatchCasePattern(index), &(pattern, body))?;
                    self.v3_pattern(&pattern_context, pattern)?;
                    let body_context =
                        context.ordered(NodeStep::PatternMatchCaseBody(index), &(pattern, body))?;
                    self.v3_value(&body_context, body)?;
                }
            }
            classic::Value::Update(_, subject, fields) => {
                self.v3_value(&context.named(NodeStep::UpdateSubject), subject)?;
                for (name, value) in fields {
                    self.v3_value(
                        &context.named(NodeStep::UpdateField(classic_name(name)?)),
                        value,
                    )?;
                }
            }
            classic::Value::Constructor(_, _)
            | classic::Value::FieldFunction(_, _)
            | classic::Value::Literal(_, _)
            | classic::Value::Unit(_)
            | classic::Value::Variable(_, _)
            | classic::Value::Reference(_, _) => {}
        }
        Ok(())
    }

    fn v3_pattern(
        &mut self,
        context: &WalkContext,
        pattern: &classic::Pattern<classic::Type<classic::Attrs>>,
    ) -> Result<(), NodeResolutionError> {
        self.add(context, IndexedNodeKind::Pattern, pattern)?;
        match pattern {
            classic::Pattern::As(_, child, _) => {
                self.v3_pattern(&context.named(NodeStep::AsPatternChild), child)?
            }
            classic::Pattern::Tuple(_, children) => {
                for (index, child) in children.iter().enumerate() {
                    let path = context.ordered(NodeStep::PatternTupleElement(index), child)?;
                    self.v3_pattern(&path, child)?;
                }
            }
            classic::Pattern::Constructor(_, _, children) => {
                for (index, child) in children.iter().enumerate() {
                    let path =
                        context.ordered(NodeStep::PatternConstructorArgument(index), child)?;
                    self.v3_pattern(&path, child)?;
                }
            }
            classic::Pattern::HeadTail(_, head, tail) => {
                self.v3_pattern(&context.named(NodeStep::HeadTailHead), head)?;
                self.v3_pattern(&context.named(NodeStep::HeadTailTail), tail)?;
            }
            classic::Pattern::Wildcard(_)
            | classic::Pattern::EmptyList(_)
            | classic::Pattern::Literal(_, _)
            | classic::Pattern::Unit(_) => {}
        }
        Ok(())
    }
}

fn parse_package(text: &str) -> Result<PackageName, NodeResolutionError> {
    let package =
        PackageName::from_canonical_string(text).map_err(NodeResolutionError::InvalidName)?;
    if package.to_canonical_string() != text || package.is_empty() {
        return Err(NodeResolutionError::InvalidName(text.to_owned()));
    }
    Ok(package)
}

fn parse_path(text: &str) -> Result<Path, NodeResolutionError> {
    let path = Path::from_canonical_string(text).map_err(NodeResolutionError::InvalidName)?;
    if path.to_canonical_string() != text || path.is_empty() {
        return Err(NodeResolutionError::InvalidName(text.to_owned()));
    }
    Ok(path)
}

fn parse_name(text: &str) -> Result<Name, NodeResolutionError> {
    let name = Name::from_canonical_string(text).map_err(NodeResolutionError::InvalidName)?;
    if name.to_canonical_string() != text {
        return Err(NodeResolutionError::InvalidName(text.to_owned()));
    }
    Ok(name)
}

fn classic_name(name: &classic::Name) -> Result<Name, NodeResolutionError> {
    let words = name
        .words
        .iter()
        .map(|word| crate::naming::resolve(*word).to_string())
        .collect::<Vec<_>>();
    let name = Name::from_words(words);
    Name::from_canonical_string(&name.to_canonical_string())
        .map_err(NodeResolutionError::InvalidName)
}

fn classic_path(path: &classic::Path) -> Result<Path, NodeResolutionError> {
    Ok(Path {
        segments: path
            .segments
            .iter()
            .map(classic_name)
            .collect::<Result<Vec<_>, _>>()?,
    })
}
