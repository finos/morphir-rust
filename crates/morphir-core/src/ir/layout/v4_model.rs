//! The v4 document tree: the [`TreeModel`] that reads a tree's files with the v4 tree-file
//! decoders and puts the pieces back together as an [`IRFile`].
//!
//! Every decode here is a thin mapping over [`serde_document`]: the decoders already know the v4
//! file shapes, spellings and diagnostics, and this model only moves what they return into the
//! layout's version-neutral types and back out again.

use indexmap::IndexMap;

use super::model::{
    AssembledModule, Entries, Envelope, ModuleFile, ModuleFileOf, Node, Packages, Role, TreeModel,
    TypeNode, ValueNode,
};
use super::paths::Root;
use crate::ir::v4::FormatVersion;
use crate::ir::v4::access::{Access, AccessControlled};
use crate::ir::v4::distribution::{
    ApplicationContent, DefinitionDependencies, Dependencies, Distribution, EntryPoints,
    LibraryContent, SpecsContent,
};
use crate::ir::v4::module::{Documented, ModuleDefinition, ModuleSpecification};
use crate::ir::v4::package::{PackageDefinition, PackageSpecification};
use crate::ir::v4::tree_files::{
    DistributionKind, DistributionManifestFile, ExpectedEntries, ModuleEntries, ModuleManifestFile,
    NodeFileBody, TypeDefinitionFile, ValueDefinitionFile,
};
use crate::ir::v4::types::{TypeDefinition, TypeSpecification};
use crate::ir::v4::value::{ValueDefinition, ValueSpecification};
use crate::ir::v4::{IRFile, serde_document};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticStage};
use crate::naming::{ModuleName, Name, PackageName};

/// The v4 document tree.
pub(crate) struct V4;

/// What a v4 distribution manifest carries beyond the envelope every version shares.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct V4Extra {
    pub format_version: FormatVersion,
    /// An application's entry points; empty for the other two kinds.
    pub entry_points: EntryPoints,
}

impl TreeModel for V4 {
    type Doc = serde_json::Value;
    type Kind = DistributionKind;
    type Extra = V4Extra;
    type TypeDef = AccessControlled<Documented<TypeDefinition>>;
    type ValueDef = AccessControlled<Documented<ValueDefinition>>;
    type TypeSpec = Documented<TypeSpecification>;
    type ValueSpec = Documented<ValueSpecification>;
    type File = IRFile;

    fn decode_manifest(
        value: &serde_json::Value,
        cursor: &str,
    ) -> Result<Envelope<DistributionKind, V4Extra>, Diagnostic> {
        let DistributionManifestFile {
            format_version,
            distribution,
            package,
            path_budget,
            dependencies,
            entry_points,
        } = serde_document::decode_distribution_manifest_file(value, cursor)?;
        Ok(Envelope {
            kind: distribution,
            package,
            path_budget,
            dependencies,
            extra: V4Extra {
                format_version,
                entry_points,
            },
        })
    }

    /// A `Specs` tree is specifications everywhere. A `Library` holds its own package's
    /// definitions and its dependencies' public faces. An `Application` links its dependencies
    /// statically, so `deps/` holds definitions there too (distributions-0010).
    fn role(kind: DistributionKind, root: Root) -> Role {
        match (kind, root) {
            (DistributionKind::Specs, _) | (DistributionKind::Library, Root::Deps) => {
                Role::Specifications
            }
            (DistributionKind::Application, Root::Deps) | (_, Root::Pkg) => Role::Definitions,
        }
    }

    fn decode_module(
        value: &serde_json::Value,
        cursor: &str,
        role: Role,
    ) -> Result<ModuleFileOf<Self>, Diagnostic> {
        let expected = match role {
            Role::Definitions => ExpectedEntries::Definitions,
            Role::Specifications => ExpectedEntries::Specifications,
        };
        let ModuleManifestFile {
            path,
            access,
            doc,
            types,
            values,
            file_names,
            ..
        } = serde_document::decode_module_manifest_file(value, cursor, expected)?;
        Ok(ModuleFile {
            path: path.into_path(),
            public: access != Access::Private,
            doc,
            types: entries(types),
            values: entries(values),
            file_names,
        })
    }

    fn decode_type_file(
        value: &serde_json::Value,
        cursor: &str,
        _role: Role,
    ) -> Result<(Name, TypeNode<Self>), Diagnostic> {
        let TypeDefinitionFile { name, body, .. } =
            serde_document::decode_type_definition_file(value, cursor)?;
        Ok((name, node(body)))
    }

    fn decode_value_file(
        value: &serde_json::Value,
        cursor: &str,
        _role: Role,
    ) -> Result<(Name, ValueNode<Self>), Diagnostic> {
        let ValueDefinitionFile { name, body, .. } =
            serde_document::decode_value_definition_file(value, cursor)?;
        Ok((name, node(body)))
    }

    /// The distribution the manifest's kind calls for, with the entry points of an `Application`
    /// taken from the manifest rather than from any file under a package root.
    fn assemble(
        envelope: Envelope<DistributionKind, V4Extra>,
        packages: Packages<Self>,
    ) -> Result<IRFile, Diagnostic> {
        let Envelope {
            kind,
            package: package_name,
            extra:
                V4Extra {
                    format_version,
                    entry_points,
                },
            ..
        } = envelope;
        let Packages { own, dependencies } = packages;
        let distribution = match kind {
            DistributionKind::Specs => Distribution::Specs(SpecsContent {
                package_name,
                spec: specification_package(own)?,
                dependencies: dependency_specifications(dependencies)?,
            }),
            DistributionKind::Library => Distribution::Library(LibraryContent {
                package_name,
                def: definition_package(own)?,
                dependencies: dependency_specifications(dependencies)?,
            }),
            DistributionKind::Application => Distribution::Application(ApplicationContent {
                package_name,
                def: definition_package(own)?,
                dependencies: dependency_definitions(dependencies)?,
                entry_points,
            }),
        };
        Ok(IRFile {
            format_version,
            distribution,
        })
    }
}

// =============================================================================
// Decoding
// =============================================================================

/// A module manifest listing, in the layout's terms.
fn entries<D, S>(listing: ModuleEntries<D, S>) -> Entries<D, S> {
    match listing {
        ModuleEntries::Names(names) => Entries::Names(names),
        ModuleEntries::Definitions(items) => Entries::Definitions(items),
        ModuleEntries::Specifications(items) => Entries::Specifications(items),
    }
}

/// A node file's body, in the layout's terms.
fn node<D, S>(body: NodeFileBody<D, S>) -> Node<D, S> {
    match body {
        NodeFileBody::Def(definition) => Node::Def(definition),
        NodeFileBody::Spec(specification) => Node::Spec(specification),
    }
}

// =============================================================================
// Assembling
// =============================================================================

type Module = AssembledModule<V4>;

fn dependency_specifications(
    dependencies: Vec<(PackageName, Vec<Module>)>,
) -> Result<Dependencies, Diagnostic> {
    let mut out = Dependencies::new();
    for (name, modules) in dependencies {
        out.insert(name.to_canonical_string(), specification_package(modules)?);
    }
    Ok(out)
}

fn dependency_definitions(
    dependencies: Vec<(PackageName, Vec<Module>)>,
) -> Result<DefinitionDependencies, Diagnostic> {
    let mut out = DefinitionDependencies::new();
    for (name, modules) in dependencies {
        out.insert(name.to_canonical_string(), definition_package(modules)?);
    }
    Ok(out)
}

fn definition_package(modules: Vec<Module>) -> Result<PackageDefinition, Diagnostic> {
    let mut out = IndexMap::with_capacity(modules.len());
    for module in modules {
        let key = ModuleName::new(module.path).to_canonical_string();
        out.insert(
            key.clone(),
            AccessControlled {
                access: if module.public {
                    Access::Public
                } else {
                    Access::Private
                },
                value: ModuleDefinition {
                    types: definitions(module.types, &key)?,
                    values: definitions(module.values, &key)?,
                    doc: module.doc,
                },
            },
        );
    }
    Ok(PackageDefinition { modules: out })
}

fn specification_package(modules: Vec<Module>) -> Result<PackageSpecification, Diagnostic> {
    let mut out = IndexMap::with_capacity(modules.len());
    for module in modules {
        let key = ModuleName::new(module.path).to_canonical_string();
        out.insert(
            key.clone(),
            ModuleSpecification {
                // A tree has nowhere to keep module annotations, so a module read out of one
                // has none; the writer refuses one that has any.
                annotations: Vec::new(),
                types: specifications(module.types, &key)?,
                values: specifications(module.values, &key)?,
                doc: module.doc,
            },
        );
    }
    Ok(PackageSpecification { modules: out })
}

/// The definitions of a module read in the definitions role.
///
/// The reader refuses a node of the other role before it ever gets here, so the refusal cannot
/// happen. It is still a refusal rather than a dropped entry: silently losing part of a module
/// would turn a defect in the reader into a distribution missing half of itself.
fn definitions<D, S>(
    entries: IndexMap<String, Node<D, S>>,
    module: &str,
) -> Result<IndexMap<String, D>, Diagnostic> {
    entries
        .into_iter()
        .map(|(key, entry)| match entry {
            Node::Def(definition) => Ok((key, definition)),
            Node::Spec(_) => Err(wrong_role(module, "a definition", "a specification")),
        })
        .collect()
}

/// The specifications of a module read in the specifications role, the other half of
/// [`definitions`].
fn specifications<D, S>(
    entries: IndexMap<String, Node<D, S>>,
    module: &str,
) -> Result<IndexMap<String, S>, Diagnostic> {
    entries
        .into_iter()
        .map(|(key, entry)| match entry {
            Node::Spec(specification) => Ok((key, specification)),
            Node::Def(_) => Err(wrong_role(module, "a specification", "a definition")),
        })
        .collect()
}

fn wrong_role(module: &str, expected: &str, found: &str) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::InvalidDistributionShape,
        DiagnosticStage::Semantic,
        module,
        format!("expected {expected} in module \"{module}\", found {found}"),
    )
}
