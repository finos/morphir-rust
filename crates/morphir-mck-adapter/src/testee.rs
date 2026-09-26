//! The operations that answer each request in the kit's adapter protocol.
//!
//! `decode` reads one node in one profile at one IR version and answers with the canonical
//! spelling of what it read, the node kind a `rejected expect=<Kind>` fence names, and the
//! legacy spellings accepted on the way (`protocol.schema.json`'s `DecodeSuccess`). The
//! spellings themselves are morphir-core's; nothing here decides what a member is called.
//!
//! # `current` and `pinned`
//!
//! The kit's two path modes are the two *module paths* a binding exposes — its newest pinned
//! version module and the current alias that points at it — and the driver requires the two to
//! agree fence by fence (kit README, "Before any of that, the driver asks the testee for its
//! capabilities"). They are not a spelling window: this binding has one set of readers, so both
//! modes decode identically, and a legacy spelling accepted under decision 0006's window warns
//! the same way on each. morphir-core's `SpellingMode::Pinned` is the mechanism that closes that
//! window at a later release, not something a path mode selects.
//!
//! # Version 3
//!
//! A version 3 node is read into the classic model and written back in the classic spelling.
//! `decode` does not migrate: the driver holds the answer against the case's own canonical
//! fence, and a case pinned to version 3 spells its canonical in version 3.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use morphir_common::ir_transport::IonCodec;
use morphir_core::ir::classic;
use morphir_core::ir::json::write_canonical;
use morphir_core::ir::v4::serde_document;
use morphir_core::ir::v4::{
    AccessControlled, Annotation, AnnotationArgument, Annotations, ApplicationContent,
    ConstructorArg, ConstructorArgSpec, ConstructorDefinition, ConstructorSpecification,
    Distribution, DistributionManifestFile, Documented, Field, FormatVersion, IRFile,
    Incompleteness, LetBinding, LibraryContent, Literal, ModuleDefinition, ModuleEntries,
    ModuleManifestFile, ModuleSpecification, NodeFileBody, PackageDefinition, PackageSpecification,
    Pattern, PatternCase, RecordFieldEntry, SpecsContent, SpellingMode, Type, TypeAttributes,
    TypeDefinition, TypeDefinitionFile, TypeEncoding, TypeSpecification, Value, ValueAttributes,
    ValueBody, ValueDefinition, ValueDefinitionFile, ValueSpecification, with_spelling_mode,
    with_type_encoding,
};
use morphir_core::ir::{Diagnostic, DiagnosticCode, Warning};
use morphir_core::naming::{FQName, Name, Path};

use crate::protocol::{
    DecodeRequest, DecodeResponse, NodeKind, PathMode, Profile, ProtocolDiagnostic,
    ReadTreeRequest, TreeFile, WriteTreeRequest, WriteTreeResponse,
};

/// The stack every decode runs on.
///
/// morphir-core's readers state their own nesting ceiling (`morphir_core::ir::json::MAX_DEPTH`,
/// `morphir_core::ir::yaml::MAX_DEPTH`) and recurse once per level, and their own document-tree
/// deserializers do too; 1000 levels of an unoptimized build's frames do not fit in the stack a
/// thread is given by default — on Windows the main thread's stack is whatever the linker
/// reserved, which is 1 MiB unless someone says otherwise. So the work runs on a thread with a
/// stack this crate states rather than inherits.
///
/// This is what one request reserves, not what a kit run reserves. [`decode`] spawns one scoped
/// thread per request and joins it before returning, so the stack is gone before the answer is
/// written: a run of hundreds of cases holds one of these at a time, never one per case. It is
/// reserved address space in any event — a shallow document commits the pages it touches and no
/// more.
const DECODE_STACK_BYTES: usize = 64 * 1024 * 1024;

fn unsupported_ion_operation(operation: &str) -> Diagnostic {
    Diagnostic::normalization(
        DiagnosticCode::UnknownNode,
        "/",
        format!("{operation} is not implemented by this adapter"),
    )
}

/// Reads one node and answers with its canonical spelling or the diagnostic that refused it.
///
/// The reading itself is [`decode_here`]; this wrapper only supplies the stack it needs (see
/// [`DECODE_STACK_BYTES`]). Scoped so the request does not have to be cloned, and the thread is
/// joined before this returns, so nothing about the answer changes.
pub fn decode(req: &DecodeRequest) -> DecodeResponse {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(DECODE_STACK_BYTES)
            .spawn_scoped(scope, || decode_here(req))
            .expect("a decode thread")
            .join()
            // A panic in the decoder is this adapter's bug, not a statement about the document,
            // and the framing loop has no way to answer one honestly. Resuming it lets the
            // process die the way it would have without the thread.
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn decode_here(req: &DecodeRequest) -> DecodeResponse {
    // This cannot happen while the driver honours `capabilities`, and it is not a statement about
    // the document, so it answers `protocol_error` rather than spending one of the kit's
    // diagnostic codes on "this binding does not do that".
    if !matches!(req.version, 3 | 4) {
        return DecodeResponse::Refused {
            diagnostic: ProtocolDiagnostic::new(format!(
                "this binding decodes IR versions 3 and 4, not {}",
                req.version
            )),
        };
    }

    match read(req) {
        Ok((node, warnings)) => {
            let node = if req.strip { node.stripped() } else { node };
            match node.write(req.profile) {
                Ok(text) => DecodeResponse::Ok {
                    kind: node.kind().to_string(),
                    canonical: BTreeMap::from([(profile_key(req.profile).to_string(), text)]),
                    warnings,
                },
                Err(diagnostic) => DecodeResponse::Err { diagnostic },
            }
        }
        Err(diagnostic) => DecodeResponse::Err { diagnostic },
    }
}

/// Reads a document tree and answers with the canonical spelling of the `IRFile` it assembles to,
/// or the diagnostic that refused it.
///
/// Runs on the same stack [`decode`] does (see [`DECODE_STACK_BYTES`]): a document tree's node
/// files go through the same nesting-sensitive readers a single document does, one file at a time.
pub fn read_tree(req: &ReadTreeRequest) -> DecodeResponse {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(DECODE_STACK_BYTES)
            .spawn_scoped(scope, || read_tree_here(req))
            .expect("a read_tree thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn read_tree_here(req: &ReadTreeRequest) -> DecodeResponse {
    if req.profile == Profile::Ion {
        return DecodeResponse::Err {
            diagnostic: unsupported_ion_operation("Ion document-tree reading"),
        };
    }
    // Not a statement about the document: this binding's document trees are version 3 and 4
    // layouts only, so an off-capabilities version answers `protocol_error` the way an
    // off-capabilities decode request does.
    if !matches!(req.version, 3 | 4) {
        return DecodeResponse::Refused {
            diagnostic: ProtocolDiagnostic::new(format!(
                "this binding reads document trees at IR versions 3 and 4, not {}",
                req.version
            )),
        };
    }

    let profile = layout_profile(req.profile);
    let files: morphir_core::ir::layout::Tree = req
        .files
        .iter()
        .map(|file| (file.path.clone(), file.content.clone()))
        .collect();

    // Version 3 is read into the classic model and answered under its own canonical spelling —
    // the tree layout is the v4 one (every file `formatVersion: "3.1.0"`), but what it assembles
    // to is a classic `Library` or `Specs`, not an `IRFile`. `req.node` is ignored here too: a
    // document tree always assembles to one distribution.
    if req.version == 3 {
        return match morphir_core::ir::layout::read_tree_v3(&files, profile) {
            Ok((distribution, warnings)) => {
                let kind = classic_distribution_kind(&distribution.distribution).to_string();
                match write_classic_canonical(&distribution, req.strip, req.profile) {
                    Ok(text) => DecodeResponse::Ok {
                        kind,
                        canonical: BTreeMap::from([(profile_key(req.profile).to_string(), text)]),
                        warnings,
                    },
                    Err(diagnostic) => DecodeResponse::Err { diagnostic },
                }
            }
            Err(diagnostic) => DecodeResponse::Err { diagnostic },
        };
    }

    match morphir_core::ir::layout::read_tree(&files, profile) {
        Ok((file, warnings)) => {
            let node = Node::IRFile(file);
            // `req.node` is ignored: a document tree always assembles to one `IRFile`, the way the
            // reference adapter's `readTreeWith` does (reference map §6.3).
            let kind = node.kind().to_string();
            let node = if req.strip { node.stripped() } else { node };
            match node.write(req.profile) {
                Ok(text) => DecodeResponse::Ok {
                    kind,
                    canonical: BTreeMap::from([(profile_key(req.profile).to_string(), text)]),
                    warnings,
                },
                Err(diagnostic) => DecodeResponse::Err { diagnostic },
            }
        }
        Err(diagnostic) => DecodeResponse::Err { diagnostic },
    }
}

/// Writes a whole distribution back out as a document tree, or answers with the diagnostic that
/// refused it.
///
/// A read failure of `req.input` is reported as the write's own failure (reference map §6.3); any
/// warnings that read produced are dropped, since a `writeTree` answer carries no `warnings`
/// member (`protocol.schema.json`'s `WriteTreeSuccess`).
pub fn write_tree(req: &WriteTreeRequest) -> WriteTreeResponse {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(DECODE_STACK_BYTES)
            .spawn_scoped(scope, || write_tree_here(req))
            .expect("a write_tree thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn write_tree_here(req: &WriteTreeRequest) -> WriteTreeResponse {
    if req.policy.profile == Profile::Ion {
        return WriteTreeResponse::Err {
            diagnostic: unsupported_ion_operation("Ion document-tree writing"),
        };
    }
    if !matches!(req.version, 3 | 4) {
        return WriteTreeResponse::Refused {
            diagnostic: ProtocolDiagnostic::new(format!(
                "this binding writes document trees at IR versions 3 and 4, not {}",
                req.version
            )),
        };
    }

    let policy = morphir_core::ir::layout::TreePolicy {
        profile: layout_profile(req.policy.profile),
        path_budget: req.policy.path_budget,
    };

    if req.version == 3 {
        let distribution = match read_whole_classic_distribution(req.policy.profile, &req.input) {
            Ok(distribution) => distribution,
            Err(diagnostic) => return WriteTreeResponse::Err { diagnostic },
        };
        return match morphir_core::ir::layout::write_tree_v3(&distribution, &policy) {
            Ok(files) => WriteTreeResponse::Ok {
                files: files
                    .into_iter()
                    .map(|(path, content)| TreeFile { path, content })
                    .collect(),
            },
            Err(diagnostic) => WriteTreeResponse::Err { diagnostic },
        };
    }

    let file = match read_whole_ir_file(req.policy.profile, &req.input) {
        Ok((file, _warnings)) => file,
        Err(diagnostic) => return WriteTreeResponse::Err { diagnostic },
    };

    match morphir_core::ir::layout::write_tree(&file, &policy) {
        Ok(files) => WriteTreeResponse::Ok {
            files: files
                .into_iter()
                .map(|(path, content)| TreeFile { path, content })
                .collect(),
        },
        Err(diagnostic) => WriteTreeResponse::Err { diagnostic },
    }
}

/// Reads a whole classic `Library` or `Specs` document (not a tree file) in the given profile,
/// the way version 3's `writeTree` carries `input`.
///
/// Version 3 has one spelling of the classic model — decision 0005's compact/expanded switch and
/// `TypeEncoding` are a v4 concept only — so this is a plain parse of the profile's own value
/// tree, the same shape [`read_v3`] reads a single classic node from.
fn read_whole_classic_distribution(
    profile: Profile,
    text: &str,
) -> Result<classic::Distribution, Diagnostic> {
    match profile {
        Profile::Json => serde_json::from_str(text).map_err(|error| recover(&error)),
        Profile::Yaml => {
            let value = morphir_core::ir::yaml::read(text)?;
            serde_json::from_value(value).map_err(|error| recover(&error))
        }
        Profile::Ion => Err(unsupported_ion_operation("v3 Ion distribution reading")),
    }
}

/// The canonical spelling of a classic distribution in the given profile, with the one trailing
/// newline a canonical fence carries — the classic-model counterpart of [`Node::write`]. With
/// `strip`, every value attribute is cleared first, as [`Node::stripped`] clears a classic value's.
fn write_classic_canonical(
    distribution: &classic::Distribution,
    strip: bool,
    profile: Profile,
) -> Result<String, Diagnostic> {
    let value = if strip {
        stripped_classic_distribution(distribution)?
    } else {
        classic_value_of(distribution)?
    };
    Ok(match profile {
        Profile::Json => format!("{}\n", write_canonical(&value)),
        Profile::Yaml => morphir_core::ir::yaml::write_canonical(&value),
        Profile::Ion => return Err(unsupported_ion_operation("v3 Ion writing")),
    })
}

fn classic_value_of<T: Serialize>(node: &T) -> Result<serde_json::Value, Diagnostic> {
    serde_json::to_value(node).map_err(|error| {
        Diagnostic::normalization(DiagnosticCode::InvalidType, "/", error.to_string())
    })
}

/// A classic distribution as a JSON value with every value attribute cleared to `{}`.
///
/// Only a `Library`'s own definitions hold value expressions: a `Specs` distribution and every
/// dependency are specifications, which carry types alone, and a classic type has nothing to clear
/// (see [`strip_classic_value`]). A `Library` types its values' attributes as the inferred type
/// itself, which has no empty spelling, so the stripped package is rebuilt with this adapter's
/// optional annotation ([`ClassicAnnotation`]) in that position and written in place of the
/// original.
fn stripped_classic_distribution(
    distribution: &classic::Distribution,
) -> Result<serde_json::Value, Diagnostic> {
    let classic::DistributionBody::Library(package, dependencies, definition) =
        &distribution.distribution
    else {
        return classic_value_of(distribution);
    };
    let mut modules = Vec::with_capacity(definition.modules.len());
    for module in &definition.modules {
        let body = &module.definition.value;
        let mut values = Vec::with_capacity(body.values.len());
        for (name, value) in &body.values {
            values.push((
                name.clone(),
                classic::AccessControlled {
                    access: value.access.clone(),
                    value: classic::Documented {
                        doc: value.value.doc.clone(),
                        value: stripped_classic_value_definition(&value.value.value)?,
                    },
                },
            ));
        }
        modules.push(classic::ModuleEntry {
            path: module.path.clone(),
            definition: classic::AccessControlled {
                access: module.definition.access.clone(),
                value: classic::ModuleDefinition::<classic::Attrs, ClassicAnnotation> {
                    types: body.types.clone(),
                    values,
                    doc: body.doc.clone(),
                },
            },
        });
    }
    let package_definition = classic::PackageDefinition { modules };
    let mut document = serde_json::Map::new();
    document.insert(
        "formatVersion".into(),
        distribution.emitted_format_version(),
    );
    document.insert(
        "distribution".into(),
        serde_json::Value::Array(vec![
            "Library".into(),
            classic_value_of(package)?,
            classic_value_of(dependencies)?,
            classic_value_of(&package_definition)?,
        ]),
    );
    Ok(serde_json::Value::Object(document))
}

/// A Library value definition, read into this adapter's optional annotation and stripped the way
/// a `decode` of the same definition with `strip` is.
fn stripped_classic_value_definition(
    definition: &classic::ValueDefinition<classic::Attrs, classic::Type<classic::Attrs>>,
) -> Result<ClassicValueDefinition, Diagnostic> {
    let written = classic_value_of(definition)?;
    let read: ClassicValueDefinition =
        serde_json::from_value(written).map_err(|error| recover(&error))?;
    Ok(strip_classic_value_definition(read))
}

/// The `kind` a version 3 `readTree` answers: the classic distribution's own variant name, the
/// way [`distribution_kind`] answers for a v4 `IRFile`.
fn classic_distribution_kind(node: &classic::DistributionBody) -> &'static str {
    match node {
        classic::DistributionBody::Library(..) => "Library",
        classic::DistributionBody::Specs(..) => "Specs",
    }
}

/// Reads a whole document (not a tree file) as an `IRFile` in the given profile, the way
/// `writeTree`'s `input` carries one.
fn read_whole_ir_file(profile: Profile, text: &str) -> Result<(IRFile, Vec<Warning>), Diagnostic> {
    match profile {
        Profile::Json => morphir_core::ir::json::read_ir_file(text).map_err(|error| error.0),
        Profile::Yaml => morphir_core::ir::yaml::read_ir_file(text).map_err(|error| error.0),
        Profile::Ion => Err(unsupported_ion_operation("Ion distribution reading")),
    }
}

/// The wire [`Profile`] as `morphir_core::ir::layout::Profile`.
fn layout_profile(profile: Profile) -> morphir_core::ir::layout::Profile {
    match profile {
        Profile::Json => morphir_core::ir::layout::Profile::Json,
        Profile::Yaml => morphir_core::ir::layout::Profile::Yaml,
        Profile::Ion => unreachable!("Ion tree operations are handled before JSON/YAML layout"),
    }
}

/// The key a canonical answer is filed under: the profile the request asked for.
fn profile_key(profile: Profile) -> &'static str {
    match profile {
        Profile::Json => "json",
        Profile::Yaml => "yaml",
        Profile::Ion => "ion",
    }
}

fn read(req: &DecodeRequest) -> Result<(Node, Vec<Warning>), Diagnostic> {
    if req.profile == Profile::Ion {
        return if req.version == 4 && req.node == NodeKind::Value {
            IonCodec::new()
                .decode_v4_value_fragment(&req.input)
                .map(|value| (Node::Value(value), Vec::new()))
                .map_err(|error| {
                    Diagnostic::normalization(DiagnosticCode::InvalidType, "/", error.to_string())
                })
        } else {
            Err(unsupported_ion_operation("this Ion node and version"))
        };
    }
    let value = match req.profile {
        // morphir-core's reader settles the repeated-member and nesting-ceiling rules before the
        // text becomes a value: `serde_json::Value` folds a repeated member onto the last one
        // written and would hide it.
        Profile::Json => morphir_core::ir::json::read(&req.input)?,
        // The YAML reader walks the document itself, so the repeated member and the nesting
        // ceiling are already its answers, in the kit's codes and with the kit's cursors; there
        // is no separate probe.
        Profile::Yaml => morphir_core::ir::yaml::read(&req.input)?,
        Profile::Ion => unreachable!("Ion is handled above"),
    };
    if req.version == 3 {
        return read_v3(req, &value).map(|node| (node, Vec::new()));
    }
    read_v4(req, value)
}

// =============================================================================
// Version 4
// =============================================================================

fn read_v4(req: &DecodeRequest, value: Json) -> Result<(Node, Vec<Warning>), Diagnostic> {
    // The separate metadata draft can read 4.1.0, while this established IR
    // suite still advertises the released support table ending before 4.1.0.
    // Keep that boundary at the adapter rather than restricting the new codec.
    let version = match req.node {
        NodeKind::FormatVersion => Some(&value),
        NodeKind::IRFile
        | NodeKind::Distribution
        | NodeKind::DistributionManifestFile
        | NodeKind::ModuleManifestFile
        | NodeKind::TypeDefinitionFile
        | NodeKind::ValueDefinitionFile => value.get("formatVersion"),
        _ => None,
    };
    if version.and_then(Json::as_str) == Some("4.1.0") {
        let cursor = if req.node == NodeKind::FormatVersion {
            "/"
        } else {
            "/formatVersion"
        };
        return Err(Diagnostic::normalization(
            DiagnosticCode::UnsupportedFormatVersionMinor,
            cursor,
            "formatVersion 4.1.0 is outside this IR suite's advertised support table",
        ));
    }
    // Both path modes read through the same readers, so both decode under the open window (see
    // the module's note on `current` and `pinned`). `req.path` is matched rather than ignored so
    // the day the two paths differ, this is where that shows up.
    let mode = match req.path {
        PathMode::Current | PathMode::Pinned => SpellingMode::Current,
    };
    let (node, warnings) = with_spelling_mode(mode, || read_v4_node(req.node, value));
    Ok((node?, warnings))
}

fn read_v4_node(kind: NodeKind, value: Json) -> Result<Node, Diagnostic> {
    fn of<T: for<'de> Deserialize<'de>>(
        value: Json,
        wrap: fn(T) -> Node,
    ) -> Result<Node, Diagnostic> {
        serde_json::from_value::<T>(value)
            .map(wrap)
            .map_err(|error| recover(&error))
    }

    match kind {
        NodeKind::Name => of(value, Node::Name),
        NodeKind::Path => of(value, Node::Path),
        NodeKind::FQName => of(value, Node::FQName),
        NodeKind::FormatVersion => of(value, Node::FormatVersion),
        NodeKind::Type => of(value, Node::Type),
        NodeKind::Literal => of(value, Node::Literal),
        NodeKind::Pattern => of(value, Node::Pattern),
        NodeKind::Value => of(value, Node::Value),
        NodeKind::TypeSpecification => of(value, Node::TypeSpecification),
        NodeKind::TypeDefinition => of(value, Node::TypeDefinition),
        NodeKind::ValueSpecification => of(value, Node::ValueSpecification),
        NodeKind::ValueDefinition => of(value, Node::ValueDefinition),
        NodeKind::AccessControlledTypeDefinition => of(value, Node::AccessControlledTypeDefinition),
        NodeKind::AccessControlledValueDefinition => {
            of(value, Node::AccessControlledValueDefinition)
        }
        NodeKind::ModuleDefinition => of(value, Node::ModuleDefinition),
        NodeKind::ModuleSpecification => of(value, Node::ModuleSpecification),
        // The kit names the whole document `Distribution` as well as `IRFile`; both spell the
        // same node, a format version beside the distribution it applies to.
        NodeKind::IRFile | NodeKind::Distribution => of(value, Node::IRFile),
        // A tree file read on its own, with no tree around it: a module manifest therefore reads
        // an inline listing as definitions, which is what the reference's own node reader does.
        NodeKind::DistributionManifestFile => of(value, Node::DistributionManifestFile),
        NodeKind::ModuleManifestFile => of(value, Node::ModuleManifestFile),
        NodeKind::TypeDefinitionFile => of(value, Node::TypeDefinitionFile),
        NodeKind::ValueDefinitionFile => of(value, Node::ValueDefinitionFile),
    }
}

/// The diagnostic a serde failure carried, or an `invalid_type` naming what serde said.
///
/// Every decoder morphir-core owns smuggles a [`Diagnostic`] through the serde error, so the
/// fallback only fires where a derived impl is still doing the reading — which is a gap in the
/// codec rather than a real answer about the document.
fn recover(error: &serde_json::Error) -> Diagnostic {
    Diagnostic::from_serde_error(error).unwrap_or_else(|| {
        Diagnostic::normalization(DiagnosticCode::InvalidType, "/", error.to_string())
    })
}

// =============================================================================
// Version 3
// =============================================================================

/// The classic value attribute: `{}` before type inference has run, the inferred type after.
///
/// Reading it as an `Attrs` rather than as a bare `Type` is what lets both kinds of classic
/// document through the same reader, and it is also what makes `strip` expressible: clearing a
/// value's attributes is `Attrs::None`, which writes back as `{}`.
type ClassicAnnotation = classic::Attrs<classic::Type<classic::Attrs>>;

/// A classic value expression as this adapter reads one.
type ClassicValue = classic::Value<classic::Attrs, ClassicAnnotation>;
type ClassicPattern = classic::Pattern<ClassicAnnotation>;
type ClassicValueDefinition = classic::ValueDefinition<classic::Attrs, ClassicAnnotation>;
type ClassicArgument = classic::value::ValueArgument<classic::Attrs, ClassicAnnotation>;

/// Reads one version 3 node in the classic model.
///
/// JSON is deserialized from the text itself, once morphir-core's reader has checked it; YAML
/// from the value morphir-core's YAML reader produced. Both profiles then write the same classic
/// value tree.
fn read_v3(req: &DecodeRequest, value: &Json) -> Result<Node, Diagnostic> {
    fn of<T: for<'de> Deserialize<'de>>(
        req: &DecodeRequest,
        value: &Json,
        wrap: impl FnOnce(T) -> Node,
    ) -> Result<Node, Diagnostic> {
        let read = match req.profile {
            Profile::Json => serde_json::from_str::<T>(&req.input),
            Profile::Yaml => T::deserialize(value),
            Profile::Ion => unreachable!("Ion is handled before v3 JSON/YAML reading"),
        };
        read.map(wrap).map_err(|error| recover(&error))
    }

    match req.node {
        NodeKind::Name => of(req, value, Node::ClassicName),
        NodeKind::Path => of(req, value, Node::ClassicPath),
        NodeKind::FQName => of(req, value, Node::ClassicFQName),
        NodeKind::Literal => of(req, value, Node::ClassicLiteral),
        NodeKind::Type => of(req, value, Node::ClassicType),
        NodeKind::Pattern => of(req, value, Node::ClassicPattern),
        NodeKind::Value => of(req, value, Node::ClassicValue),
        NodeKind::ValueDefinition => of(req, value, Node::ClassicValueDefinition),
        NodeKind::TypeSpecification => of(req, value, Node::ClassicTypeSpecification),
        // A version 3 format version is one of the support table's v3 releases; the table itself
        // is the v4 node's, so a later minor is refused the same way at either version.
        NodeKind::FormatVersion => {
            v3_major(value, "/")?;
            of(req, value, Node::FormatVersion)
        }
        // The kit names the whole document `Distribution` as well as `IRFile`.
        NodeKind::IRFile | NodeKind::Distribution => {
            // A document of another version is that before anything else: its members are its
            // own version's business (document-tree-0016).
            if let Some(version) = value.get("formatVersion") {
                v3_major(version, "/formatVersion")?;
            }
            // The classic reader drops a root member it does not know and has no code for a
            // missing version, so the root is held to the v4 document's rule first.
            serde_document::document_root(value, "", "a version 3 document")?;
            of(req, value, |distribution| Node::ClassicDistribution {
                distribution,
                strip: false,
            })
        }
        NodeKind::DistributionManifestFile => {
            morphir_core::ir::layout::read_v3_manifest_file(value).map(Node::ClassicManifestFile)
        }
        // These are nodes a classic document carries only inside a whole distribution, or the
        // tree files whose role (definitions or specifications) only the tree around them
        // decides — so there is no version 3 answer this adapter gives for them on their own.
        NodeKind::TypeDefinition
        | NodeKind::ValueSpecification
        | NodeKind::AccessControlledTypeDefinition
        | NodeKind::AccessControlledValueDefinition
        | NodeKind::ModuleDefinition
        | NodeKind::ModuleSpecification
        | NodeKind::ModuleManifestFile
        | NodeKind::TypeDefinitionFile
        | NodeKind::ValueDefinitionFile => Err(Diagnostic::normalization(
            DiagnosticCode::UnknownNode,
            "/",
            format!(
                "{:?} is not a node this binding reads on its own at version 3",
                req.node
            ),
        )),
    }
}

/// Refuses a format version whose major is not 3 as a document of another version. Anything that
/// has no readable major is left to the reader, which says what is wrong with it.
fn v3_major(version: &Json, cursor: &str) -> Result<(), Diagnostic> {
    let major = match version {
        Json::Number(number) => number.as_u64(),
        Json::String(text) => text.split('.').next().and_then(|major| major.parse().ok()),
        _ => None,
    };
    match major {
        Some(major) if major != 3 => Err(Diagnostic::normalization(
            DiagnosticCode::VersionMismatch,
            cursor,
            format!("formatVersion {version} is not a version 3 release"),
        )),
        _ => Ok(()),
    }
}

/// Clearing a classic value's attributes is writing `{}` in each attribute position — which is
/// the same thing an untyped classic document already says.
///
/// A classic *type* attribute is `Attrs<()>`, and no input the profile admits decodes it as
/// anything but `Attrs::None`, so a type carries nothing to clear and is returned as it came.
fn strip_classic_value(node: ClassicValue) -> ClassicValue {
    let attributes = classic::Attrs::None;
    match node {
        classic::Value::Apply(_, function, argument) => classic::Value::Apply(
            attributes,
            Box::new(strip_classic_value(*function)),
            Box::new(strip_classic_value(*argument)),
        ),
        classic::Value::Constructor(_, name) => classic::Value::Constructor(attributes, name),
        classic::Value::Destructure(_, pattern, subject, body) => classic::Value::Destructure(
            attributes,
            strip_classic_pattern(pattern),
            Box::new(strip_classic_value(*subject)),
            Box::new(strip_classic_value(*body)),
        ),
        classic::Value::Field(_, target, name) => {
            classic::Value::Field(attributes, Box::new(strip_classic_value(*target)), name)
        }
        classic::Value::FieldFunction(_, name) => classic::Value::FieldFunction(attributes, name),
        classic::Value::IfThenElse(_, condition, then_branch, else_branch) => {
            classic::Value::IfThenElse(
                attributes,
                Box::new(strip_classic_value(*condition)),
                Box::new(strip_classic_value(*then_branch)),
                Box::new(strip_classic_value(*else_branch)),
            )
        }
        classic::Value::Lambda(_, pattern, body) => classic::Value::Lambda(
            attributes,
            strip_classic_pattern(pattern),
            Box::new(strip_classic_value(*body)),
        ),
        classic::Value::LetDefinition(_, name, definition, body) => classic::Value::LetDefinition(
            attributes,
            name,
            Box::new(strip_classic_value_definition(*definition)),
            Box::new(strip_classic_value(*body)),
        ),
        classic::Value::LetRecursion(_, bindings, body) => classic::Value::LetRecursion(
            attributes,
            bindings
                .into_iter()
                .map(|(name, definition)| {
                    (name, Box::new(strip_classic_value_definition(*definition)))
                })
                .collect(),
            Box::new(strip_classic_value(*body)),
        ),
        classic::Value::List(_, values) => classic::Value::List(
            attributes,
            values.into_iter().map(strip_classic_value).collect(),
        ),
        classic::Value::Literal(_, literal) => classic::Value::Literal(attributes, literal),
        classic::Value::PatternMatch(_, subject, cases) => classic::Value::PatternMatch(
            attributes,
            Box::new(strip_classic_value(*subject)),
            cases
                .into_iter()
                .map(|(pattern, body)| (strip_classic_pattern(pattern), strip_classic_value(body)))
                .collect(),
        ),
        classic::Value::Record(_, fields) => classic::Value::Record(
            attributes,
            fields
                .into_iter()
                .map(|(name, value)| (name, strip_classic_value(value)))
                .collect(),
        ),
        classic::Value::Tuple(_, values) => classic::Value::Tuple(
            attributes,
            values.into_iter().map(strip_classic_value).collect(),
        ),
        classic::Value::Unit(_) => classic::Value::Unit(attributes),
        classic::Value::Update(_, record, fields) => classic::Value::Update(
            attributes,
            Box::new(strip_classic_value(*record)),
            fields
                .into_iter()
                .map(|(name, value)| (name, strip_classic_value(value)))
                .collect(),
        ),
        classic::Value::Variable(_, name) => classic::Value::Variable(attributes, name),
        classic::Value::Reference(_, name) => classic::Value::Reference(attributes, name),
    }
}

fn strip_classic_pattern(node: ClassicPattern) -> ClassicPattern {
    let attributes = classic::Attrs::None;
    match node {
        classic::Pattern::Wildcard(_) => classic::Pattern::Wildcard(attributes),
        classic::Pattern::As(_, pattern, name) => {
            classic::Pattern::As(attributes, Box::new(strip_classic_pattern(*pattern)), name)
        }
        classic::Pattern::Tuple(_, patterns) => classic::Pattern::Tuple(
            attributes,
            patterns.into_iter().map(strip_classic_pattern).collect(),
        ),
        classic::Pattern::Constructor(_, name, arguments) => classic::Pattern::Constructor(
            attributes,
            name,
            arguments.into_iter().map(strip_classic_pattern).collect(),
        ),
        classic::Pattern::EmptyList(_) => classic::Pattern::EmptyList(attributes),
        classic::Pattern::HeadTail(_, head, tail) => classic::Pattern::HeadTail(
            attributes,
            Box::new(strip_classic_pattern(*head)),
            Box::new(strip_classic_pattern(*tail)),
        ),
        classic::Pattern::Literal(_, literal) => classic::Pattern::Literal(attributes, literal),
        classic::Pattern::Unit(_) => classic::Pattern::Unit(attributes),
    }
}

fn strip_classic_argument(argument: ClassicArgument) -> ClassicArgument {
    classic::value::ValueArgument {
        name: argument.name,
        annotation: classic::Attrs::None,
        ty: argument.ty,
    }
}

fn strip_classic_value_definition(definition: ClassicValueDefinition) -> ClassicValueDefinition {
    classic::ValueDefinition {
        input_types: definition
            .input_types
            .into_iter()
            .map(strip_classic_argument)
            .collect(),
        output_type: definition.output_type,
        body: strip_classic_value(definition.body),
    }
}

fn classic_literal_kind(literal: &classic::Literal) -> &'static str {
    match literal {
        classic::Literal::Bool(_) => "BoolLiteral",
        classic::Literal::Char(_) => "CharLiteral",
        classic::Literal::String(_) => "StringLiteral",
        classic::Literal::WholeNumber(_) => "WholeNumberLiteral",
        classic::Literal::Float(_) => "FloatLiteral",
        classic::Literal::Decimal(_) => "DecimalLiteral",
    }
}

fn classic_type_kind(node: &classic::Type<classic::Attrs>) -> &'static str {
    match node {
        classic::Type::ExtensibleRecord(..) => "ExtensibleRecord",
        classic::Type::Function(..) => "Function",
        classic::Type::Record(..) => "Record",
        classic::Type::Reference(..) => "Reference",
        classic::Type::Tuple(..) => "Tuple",
        classic::Type::Unit(_) => "Unit",
        classic::Type::Variable(..) => "Variable",
    }
}

fn classic_type_specification_kind(
    node: &classic::TypeSpecification<classic::Attrs>,
) -> &'static str {
    match node {
        classic::TypeSpecification::Alias(..) => "TypeAliasSpecification",
        classic::TypeSpecification::Opaque(_) => "OpaqueTypeSpecification",
        classic::TypeSpecification::Custom(..) => "CustomTypeSpecification",
        classic::TypeSpecification::Derived(..) => "DerivedTypeSpecification",
    }
}

fn classic_pattern_kind(node: &ClassicPattern) -> &'static str {
    match node {
        classic::Pattern::Wildcard(_) => "WildcardPattern",
        classic::Pattern::As(..) => "AsPattern",
        classic::Pattern::Tuple(..) => "TuplePattern",
        classic::Pattern::Constructor(..) => "ConstructorPattern",
        classic::Pattern::EmptyList(_) => "EmptyListPattern",
        classic::Pattern::HeadTail(..) => "HeadTailPattern",
        classic::Pattern::Literal(..) => "LiteralPattern",
        classic::Pattern::Unit(_) => "UnitPattern",
    }
}

fn classic_value_kind(node: &ClassicValue) -> &'static str {
    match node {
        classic::Value::Apply(..) => "Apply",
        classic::Value::Constructor(..) => "Constructor",
        classic::Value::Destructure(..) => "Destructure",
        classic::Value::Field(..) => "Field",
        classic::Value::FieldFunction(..) => "FieldFunction",
        classic::Value::IfThenElse(..) => "IfThenElse",
        classic::Value::Lambda(..) => "Lambda",
        classic::Value::LetDefinition(..) => "LetDefinition",
        classic::Value::LetRecursion(..) => "LetRecursion",
        classic::Value::List(..) => "List",
        classic::Value::Literal(..) => "Literal",
        classic::Value::PatternMatch(..) => "PatternMatch",
        classic::Value::Record(..) => "Record",
        classic::Value::Tuple(..) => "Tuple",
        classic::Value::Unit(_) => "Unit",
        classic::Value::Update(..) => "UpdateRecord",
        classic::Value::Variable(..) => "Variable",
        classic::Value::Reference(..) => "Reference",
    }
}

// =============================================================================
// The decoded node
// =============================================================================

/// One decoded node of any kind the kit names.
#[derive(Debug, Clone)]
enum Node {
    Name(Name),
    Path(Path),
    FQName(FQName),
    FormatVersion(FormatVersion),
    Type(Type),
    Literal(Literal),
    Pattern(Pattern),
    Value(Value),
    TypeSpecification(TypeSpecification),
    TypeDefinition(TypeDefinition),
    ValueSpecification(ValueSpecification),
    ValueDefinition(ValueDefinition),
    AccessControlledTypeDefinition(AccessControlled<Documented<TypeDefinition>>),
    AccessControlledValueDefinition(AccessControlled<Documented<ValueDefinition>>),
    ModuleDefinition(ModuleDefinition),
    ModuleSpecification(ModuleSpecification),
    IRFile(IRFile),
    // The four files a document tree is made of. Each is a node in its own right, so it is read,
    // stripped and written here the way every other node is.
    DistributionManifestFile(DistributionManifestFile),
    ModuleManifestFile(ModuleManifestFile),
    TypeDefinitionFile(TypeDefinitionFile),
    ValueDefinitionFile(ValueDefinitionFile),
    // Version 3 stays in the classic model: it is read, stripped and written there, so a case
    // pinned to version 3 is answered in the spelling its own canonical fence uses.
    ClassicName(classic::Name),
    ClassicPath(classic::Path),
    ClassicFQName(classic::FQName),
    ClassicLiteral(classic::Literal),
    ClassicType(classic::Type<classic::Attrs>),
    ClassicTypeSpecification(classic::TypeSpecification<classic::Attrs>),
    ClassicPattern(ClassicPattern),
    ClassicValue(ClassicValue),
    ClassicValueDefinition(ClassicValueDefinition),
    /// A whole classic document. Its attributes are cleared as it is written rather than in the
    /// model, whose `Library` value attribute is the inferred type itself (see
    /// [`stripped_classic_distribution`]).
    ClassicDistribution {
        distribution: classic::Distribution,
        strip: bool,
    },
    /// A v3 tree's manifest, as the value a v3 tree writes for it. A manifest has no attributes.
    ClassicManifestFile(Json),
}

impl Node {
    /// The name a `rejected expect=<Kind>` fence means: the variant the node decoded to, or the
    /// node's own name where it has no variants.
    fn kind(&self) -> &'static str {
        match self {
            Node::Name(_) => "Name",
            Node::Path(_) => "Path",
            Node::FQName(_) => "FQName",
            Node::FormatVersion(_) => "FormatVersion",
            Node::Type(node) => type_kind(node),
            Node::Literal(node) => literal_kind(node),
            Node::Pattern(node) => pattern_kind(node),
            Node::Value(node) => value_kind(node),
            Node::TypeSpecification(node) => type_specification_kind(node),
            Node::TypeDefinition(node) => type_definition_kind(node),
            Node::ValueSpecification(_) => "ValueSpecification",
            Node::ValueDefinition(node) => value_definition_kind(node),
            Node::AccessControlledTypeDefinition(node) => type_definition_kind(&node.value.value),
            Node::AccessControlledValueDefinition(node) => value_definition_kind(&node.value.value),
            Node::ModuleDefinition(_) => "ModuleDefinition",
            Node::ModuleSpecification(_) => "ModuleSpecification",
            Node::IRFile(node) => distribution_kind(&node.distribution),
            // A manifest has no variants: it is the file kind itself.
            Node::DistributionManifestFile(_) => "DistributionManifestFile",
            Node::ModuleManifestFile(_) => "ModuleManifestFile",
            // A node file answers with what is inside it, the way a bare definition or
            // specification node does.
            Node::TypeDefinitionFile(node) => match &node.body {
                NodeFileBody::Def(definition) => type_definition_kind(&definition.value.value),
                NodeFileBody::Spec(specification) => type_specification_kind(&specification.value),
            },
            // A value specification has no variants, so a spec file answers with the name of what
            // it holds, the way a bare ValueSpecification node does.
            Node::ValueDefinitionFile(node) => match &node.body {
                NodeFileBody::Def(definition) => value_definition_kind(&definition.value.value),
                NodeFileBody::Spec(_) => "ValueSpecification",
            },
            Node::ClassicName(_) => "Name",
            Node::ClassicPath(_) => "Path",
            Node::ClassicFQName(_) => "FQName",
            Node::ClassicLiteral(node) => classic_literal_kind(node),
            Node::ClassicType(node) => classic_type_kind(node),
            Node::ClassicTypeSpecification(node) => classic_type_specification_kind(node),
            Node::ClassicPattern(node) => classic_pattern_kind(node),
            Node::ClassicValue(node) => classic_value_kind(node),
            Node::ClassicValueDefinition(_) => "ValueDefinition",
            Node::ClassicDistribution { distribution, .. } => {
                classic_distribution_kind(&distribution.distribution)
            }
            Node::ClassicManifestFile(_) => "DistributionManifestFile",
        }
    }

    /// The node with every attribute cleared, so two spellings are compared on meaning alone.
    fn stripped(self) -> Node {
        match self {
            Node::Type(node) => Node::Type(strip_type(node)),
            Node::Pattern(node) => Node::Pattern(strip_pattern(node)),
            Node::Value(node) => Node::Value(strip_value(node)),
            Node::TypeSpecification(node) => {
                Node::TypeSpecification(strip_type_specification(node))
            }
            Node::TypeDefinition(node) => Node::TypeDefinition(strip_type_definition(node)),
            Node::ValueSpecification(node) => {
                Node::ValueSpecification(strip_value_specification(node))
            }
            Node::ValueDefinition(node) => Node::ValueDefinition(strip_value_definition(node)),
            Node::AccessControlledTypeDefinition(node) => {
                Node::AccessControlledTypeDefinition(strip_access_controlled(node, |d| {
                    strip_documented(d, strip_type_definition)
                }))
            }
            Node::AccessControlledValueDefinition(node) => {
                Node::AccessControlledValueDefinition(strip_access_controlled(node, |d| {
                    strip_documented(d, strip_value_definition)
                }))
            }
            Node::ModuleDefinition(node) => Node::ModuleDefinition(strip_module_definition(node)),
            Node::ModuleSpecification(node) => {
                Node::ModuleSpecification(strip_module_specification(node))
            }
            Node::IRFile(node) => Node::IRFile(IRFile {
                format_version: node.format_version,
                metadata: node.metadata,
                distribution: strip_distribution(node.distribution),
            }),
            // A distribution manifest is names, a kind and a budget: nothing it holds carries
            // attributes, so it is returned as it came (the `other` arm below would do the same,
            // but saying so here keeps the four file kinds together).
            Node::DistributionManifestFile(node) => Node::DistributionManifestFile(node),
            Node::ModuleManifestFile(node) => Node::ModuleManifestFile(ModuleManifestFile {
                types: strip_module_entries(node.types, strip_access_controlled_type_definition, {
                    |specification| strip_documented(specification, strip_type_specification)
                }),
                values: strip_module_entries(
                    node.values,
                    strip_access_controlled_value_definition,
                    |specification| strip_documented(specification, strip_value_specification),
                ),
                ..node
            }),
            Node::TypeDefinitionFile(node) => Node::TypeDefinitionFile(TypeDefinitionFile {
                body: match node.body {
                    NodeFileBody::Def(definition) => {
                        NodeFileBody::Def(strip_access_controlled_type_definition(definition))
                    }
                    NodeFileBody::Spec(specification) => NodeFileBody::Spec(strip_documented(
                        specification,
                        strip_type_specification,
                    )),
                },
                ..node
            }),
            Node::ValueDefinitionFile(node) => Node::ValueDefinitionFile(ValueDefinitionFile {
                body: match node.body {
                    NodeFileBody::Def(definition) => {
                        NodeFileBody::Def(strip_access_controlled_value_definition(definition))
                    }
                    NodeFileBody::Spec(specification) => NodeFileBody::Spec(strip_documented(
                        specification,
                        strip_value_specification,
                    )),
                },
                ..node
            }),
            Node::ClassicPattern(node) => Node::ClassicPattern(strip_classic_pattern(node)),
            Node::ClassicValue(node) => Node::ClassicValue(strip_classic_value(node)),
            Node::ClassicValueDefinition(node) => {
                Node::ClassicValueDefinition(strip_classic_value_definition(node))
            }
            Node::ClassicDistribution { distribution, .. } => Node::ClassicDistribution {
                distribution,
                strip: true,
            },
            // Names, paths, literals, the format version, a classic type and a classic type
            // specification — which carries nothing but types — have no attributes a reader can
            // clear.
            other => other,
        }
    }

    /// The canonical spelling of this node in the requested profile, in the compact type
    /// encoding, with the one trailing newline a canonical fence carries.
    ///
    /// Both profiles write the same value tree — the node's compact serialisation — so a case's
    /// two canonical fences are two spellings of one answer rather than two answers.
    fn write(&self, profile: Profile) -> Result<String, Diagnostic> {
        if profile == Profile::Ion {
            return match self {
                Node::Value(value) => {
                    IonCodec::new()
                        .encode_v4_value_fragment(value)
                        .map_err(|error| {
                            Diagnostic::normalization(
                                DiagnosticCode::InvalidType,
                                "/",
                                error.to_string(),
                            )
                        })
                }
                _ => Err(unsupported_ion_operation("this Ion node")),
            };
        }
        let value = self.value()?;
        Ok(match profile {
            Profile::Json => format!("{}\n", write_canonical(&value)),
            // The YAML writer ends its output with the newline itself.
            Profile::Yaml => morphir_core::ir::yaml::write_canonical(&value),
            Profile::Ion => unreachable!("Ion is handled above"),
        })
    }

    /// This node as the value tree both canonical writers spell, in the compact type encoding.
    fn value(&self) -> Result<Json, Diagnostic> {
        fn text<T: Serialize>(node: &T) -> Result<Json, Diagnostic> {
            with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(node)).map_err(
                |error| {
                    Diagnostic::normalization(DiagnosticCode::InvalidType, "/", error.to_string())
                },
            )
        }

        match self {
            Node::Name(node) => text(node),
            Node::Path(node) => text(node),
            Node::FQName(node) => text(node),
            Node::FormatVersion(node) => text(node),
            Node::Type(node) => text(node),
            Node::Literal(node) => text(node),
            Node::Pattern(node) => text(node),
            Node::Value(node) => text(node),
            Node::TypeSpecification(node) => text(node),
            Node::TypeDefinition(node) => text(node),
            Node::ValueSpecification(node) => text(node),
            Node::ValueDefinition(node) => text(node),
            Node::AccessControlledTypeDefinition(node) => text(node),
            Node::AccessControlledValueDefinition(node) => text(node),
            Node::ModuleDefinition(node) => text(node),
            Node::ModuleSpecification(node) => text(node),
            // `formatVersion` first, then `distribution`: the root member order of the file.
            Node::IRFile(node) => text(node),
            Node::DistributionManifestFile(node) => text(node),
            Node::ModuleManifestFile(node) => text(node),
            Node::TypeDefinitionFile(node) => text(node),
            Node::ValueDefinitionFile(node) => text(node),
            Node::ClassicName(node) => text(node),
            Node::ClassicPath(node) => text(node),
            Node::ClassicFQName(node) => text(node),
            Node::ClassicLiteral(node) => text(node),
            Node::ClassicType(node) => text(node),
            Node::ClassicTypeSpecification(node) => text(node),
            Node::ClassicPattern(node) => text(node),
            Node::ClassicValue(node) => text(node),
            Node::ClassicValueDefinition(node) => text(node),
            Node::ClassicDistribution {
                distribution,
                strip: true,
            } => stripped_classic_distribution(distribution),
            Node::ClassicDistribution {
                distribution,
                strip: false,
            } => classic_value_of(distribution),
            Node::ClassicManifestFile(node) => Ok(node.clone()),
        }
    }
}

fn type_kind(node: &Type) -> &'static str {
    match node {
        Type::Variable(..) => "Variable",
        Type::Reference(..) => "Reference",
        Type::Tuple(..) => "Tuple",
        Type::Record(..) => "Record",
        Type::ExtensibleRecord(..) => "ExtensibleRecord",
        Type::Function(..) => "Function",
        Type::Unit(..) => "Unit",
    }
}

fn literal_kind(node: &Literal) -> &'static str {
    match node {
        Literal::Bool(_) => "BoolLiteral",
        Literal::Char(_) => "CharLiteral",
        Literal::String(_) => "StringLiteral",
        Literal::Integer(_) => "IntegerLiteral",
        Literal::Float(_) => "FloatLiteral",
        Literal::Decimal(_) => "DecimalLiteral",
        Literal::Document(_) => "DocumentLiteral",
    }
}

fn pattern_kind(node: &Pattern) -> &'static str {
    match node {
        Pattern::WildcardPattern(_) => "WildcardPattern",
        Pattern::AsPattern(..) => "AsPattern",
        Pattern::TuplePattern(..) => "TuplePattern",
        Pattern::ConstructorPattern(..) => "ConstructorPattern",
        Pattern::EmptyListPattern(_) => "EmptyListPattern",
        Pattern::HeadTailPattern(..) => "HeadTailPattern",
        Pattern::LiteralPattern(..) => "LiteralPattern",
        Pattern::UnitPattern(_) => "UnitPattern",
    }
}

fn value_kind(node: &Value) -> &'static str {
    match node {
        Value::Literal(..) => "Literal",
        Value::Constructor(..) => "Constructor",
        Value::Tuple(..) => "Tuple",
        Value::List(..) => "List",
        Value::Record(..) => "Record",
        Value::Variable(..) => "Variable",
        Value::Reference(..) => "Reference",
        Value::Field(..) => "Field",
        Value::FieldFunction(..) => "FieldFunction",
        Value::Apply(..) => "Apply",
        Value::Lambda(..) => "Lambda",
        Value::LetDefinition(..) => "LetDefinition",
        Value::LetRecursion(..) => "LetRecursion",
        Value::Destructure(..) => "Destructure",
        Value::IfThenElse(..) => "IfThenElse",
        Value::PatternMatch(..) => "PatternMatch",
        Value::UpdateRecord(..) => "UpdateRecord",
        Value::Unit(_) => "Unit",
        Value::Hole(..) => "Hole",
    }
}

fn type_specification_kind(node: &TypeSpecification) -> &'static str {
    match node {
        TypeSpecification::TypeAliasSpecification { .. } => "TypeAliasSpecification",
        TypeSpecification::OpaqueTypeSpecification { .. } => "OpaqueTypeSpecification",
        TypeSpecification::CustomTypeSpecification { .. } => "CustomTypeSpecification",
        TypeSpecification::DerivedTypeSpecification { .. } => "DerivedTypeSpecification",
    }
}

fn type_definition_kind(node: &TypeDefinition) -> &'static str {
    match node {
        TypeDefinition::TypeAliasDefinition { .. } => "TypeAliasDefinition",
        TypeDefinition::CustomTypeDefinition { .. } => "CustomTypeDefinition",
        TypeDefinition::IncompleteTypeDefinition { .. } => "IncompleteTypeDefinition",
    }
}

fn value_definition_kind(node: &ValueDefinition) -> &'static str {
    match node.body {
        ValueBody::Expression(_) => "ExpressionBody",
        ValueBody::Native { .. } => "NativeBody",
        ValueBody::External { .. } => "ExternalBody",
        ValueBody::Incomplete { .. } => "IncompleteBody",
    }
}

fn distribution_kind(node: &Distribution) -> &'static str {
    match node {
        Distribution::Library(_) => "Library",
        Distribution::Specs(_) => "Specs",
        Distribution::Application(_) => "Application",
    }
}

// =============================================================================
// Clearing attributes
// =============================================================================

fn strip_type(node: Type) -> Type {
    let attributes = TypeAttributes::default();
    match node {
        Type::Variable(_, name) => Type::Variable(attributes, name),
        Type::Reference(_, fqname, arguments) => Type::Reference(
            attributes,
            fqname,
            arguments.into_iter().map(strip_type).collect(),
        ),
        Type::Tuple(_, elements) => {
            Type::Tuple(attributes, elements.into_iter().map(strip_type).collect())
        }
        Type::Record(_, fields) => {
            Type::Record(attributes, fields.into_iter().map(strip_field).collect())
        }
        Type::ExtensibleRecord(_, name, fields) => Type::ExtensibleRecord(
            attributes,
            name,
            fields.into_iter().map(strip_field).collect(),
        ),
        Type::Function(_, parameter, result) => Type::Function(
            attributes,
            Box::new(strip_type(*parameter)),
            Box::new(strip_type(*result)),
        ),
        Type::Unit(_) => Type::Unit(attributes),
    }
}

fn strip_field(field: Field) -> Field {
    Field {
        name: field.name,
        tpe: strip_type(field.tpe),
    }
}

fn strip_pattern(node: Pattern) -> Pattern {
    let attributes = ValueAttributes::default();
    match node {
        Pattern::WildcardPattern(_) => Pattern::WildcardPattern(attributes),
        Pattern::AsPattern(_, pattern, name) => {
            Pattern::AsPattern(attributes, Box::new(strip_pattern(*pattern)), name)
        }
        Pattern::TuplePattern(_, patterns) => Pattern::TuplePattern(
            attributes,
            patterns.into_iter().map(strip_pattern).collect(),
        ),
        Pattern::ConstructorPattern(_, fqname, arguments) => Pattern::ConstructorPattern(
            attributes,
            fqname,
            arguments.into_iter().map(strip_pattern).collect(),
        ),
        Pattern::EmptyListPattern(_) => Pattern::EmptyListPattern(attributes),
        Pattern::HeadTailPattern(_, head, tail) => Pattern::HeadTailPattern(
            attributes,
            Box::new(strip_pattern(*head)),
            Box::new(strip_pattern(*tail)),
        ),
        Pattern::LiteralPattern(_, literal) => Pattern::LiteralPattern(attributes, literal),
        Pattern::UnitPattern(_) => Pattern::UnitPattern(attributes),
    }
}

fn strip_value(node: Value) -> Value {
    let attributes = ValueAttributes::default();
    match node {
        Value::Literal(_, literal) => Value::Literal(attributes, literal),
        Value::Constructor(_, fqname) => Value::Constructor(attributes, fqname),
        Value::Tuple(_, values) => {
            Value::Tuple(attributes, values.into_iter().map(strip_value).collect())
        }
        Value::List(_, values) => {
            Value::List(attributes, values.into_iter().map(strip_value).collect())
        }
        Value::Record(_, fields) => Value::Record(
            attributes,
            fields.into_iter().map(strip_record_field).collect(),
        ),
        Value::Variable(_, name) => Value::Variable(attributes, name),
        Value::Reference(_, fqname) => Value::Reference(attributes, fqname),
        Value::Field(_, target, name) => {
            Value::Field(attributes, Box::new(strip_value(*target)), name)
        }
        Value::FieldFunction(_, name) => Value::FieldFunction(attributes, name),
        Value::Apply(_, function, argument) => Value::Apply(
            attributes,
            Box::new(strip_value(*function)),
            Box::new(strip_value(*argument)),
        ),
        Value::Lambda(_, pattern, body) => Value::Lambda(
            attributes,
            strip_pattern(pattern),
            Box::new(strip_value(*body)),
        ),
        Value::LetDefinition(_, name, definition, body) => Value::LetDefinition(
            attributes,
            name,
            Box::new(strip_value_definition(*definition)),
            Box::new(strip_value(*body)),
        ),
        Value::LetRecursion(_, bindings, body) => Value::LetRecursion(
            attributes,
            bindings
                .into_iter()
                .map(|binding| LetBinding(binding.0, strip_value_definition(binding.1)))
                .collect(),
            Box::new(strip_value(*body)),
        ),
        Value::Destructure(_, pattern, subject, body) => Value::Destructure(
            attributes,
            strip_pattern(pattern),
            Box::new(strip_value(*subject)),
            Box::new(strip_value(*body)),
        ),
        Value::IfThenElse(_, condition, then_branch, else_branch) => Value::IfThenElse(
            attributes,
            Box::new(strip_value(*condition)),
            Box::new(strip_value(*then_branch)),
            Box::new(strip_value(*else_branch)),
        ),
        Value::PatternMatch(_, subject, cases) => Value::PatternMatch(
            attributes,
            Box::new(strip_value(*subject)),
            cases
                .into_iter()
                .map(|case| PatternCase(strip_pattern(case.0), strip_value(case.1)))
                .collect(),
        ),
        Value::UpdateRecord(_, target, fields) => Value::UpdateRecord(
            attributes,
            Box::new(strip_value(*target)),
            fields.into_iter().map(strip_record_field).collect(),
        ),
        Value::Unit(_) => Value::Unit(attributes),
        Value::Hole(_, reason, expected_type) => Value::Hole(
            attributes,
            reason,
            expected_type.map(|t| Box::new(strip_type(*t))),
        ),
    }
}

fn strip_record_field(field: RecordFieldEntry) -> RecordFieldEntry {
    RecordFieldEntry(field.0, strip_value(field.1))
}

/// Strips the attributes off the value expressions an annotation's arguments carry; the names and
/// the free text of an annotation carry none.
fn strip_annotations(mut annotations: Annotations) -> Annotations {
    annotations.entries = annotations
        .entries
        .into_iter()
        .map(|annotation| match annotation {
            Annotation::Compact { name, text } => Annotation::Compact { name, text },
            Annotation::Structured { name, args } => Annotation::Structured {
                name,
                args: args
                    .into_iter()
                    .map(|argument| match argument {
                        AnnotationArgument::Positional(value) => {
                            AnnotationArgument::Positional(strip_value(value))
                        }
                        AnnotationArgument::Named { name, value } => AnnotationArgument::Named {
                            name,
                            value: strip_value(value),
                        },
                    })
                    .collect(),
            },
            Annotation::LinkedCompact {
                authored_name,
                declaration,
            } => Annotation::LinkedCompact {
                authored_name,
                declaration,
            },
            Annotation::LinkedStructured {
                authored_name,
                declaration,
                args,
            } => Annotation::LinkedStructured {
                authored_name,
                declaration,
                args: args
                    .into_iter()
                    .map(|argument| match argument {
                        AnnotationArgument::Positional(value) => {
                            AnnotationArgument::Positional(strip_value(value))
                        }
                        AnnotationArgument::Named { name, value } => AnnotationArgument::Named {
                            name,
                            value: strip_value(value),
                        },
                    })
                    .collect(),
            },
            Annotation::PendingCompact { authored_name } => {
                Annotation::PendingCompact { authored_name }
            }
            Annotation::PendingStructured {
                authored_name,
                args,
            } => Annotation::PendingStructured {
                authored_name,
                args: args
                    .into_iter()
                    .map(|argument| match argument {
                        AnnotationArgument::Positional(value) => {
                            AnnotationArgument::Positional(strip_value(value))
                        }
                        AnnotationArgument::Named { name, value } => AnnotationArgument::Named {
                            name,
                            value: strip_value(value),
                        },
                    })
                    .collect(),
            },
        })
        .collect();
    annotations
}

fn strip_type_specification(node: TypeSpecification) -> TypeSpecification {
    match node {
        TypeSpecification::TypeAliasSpecification {
            annotations,
            type_params,
            type_expr,
        } => TypeSpecification::TypeAliasSpecification {
            annotations: strip_annotations(annotations),
            type_params,
            type_expr: strip_type(type_expr),
        },
        TypeSpecification::OpaqueTypeSpecification {
            annotations,
            type_params,
        } => TypeSpecification::OpaqueTypeSpecification {
            annotations: strip_annotations(annotations),
            type_params,
        },
        TypeSpecification::CustomTypeSpecification {
            annotations,
            type_params,
            constructors,
        } => TypeSpecification::CustomTypeSpecification {
            annotations: strip_annotations(annotations),
            type_params,
            constructors: constructors
                .into_iter()
                .map(|constructor| ConstructorSpecification {
                    name: constructor.name,
                    args: constructor
                        .args
                        .into_iter()
                        .map(|arg| ConstructorArgSpec {
                            name: arg.name,
                            arg_type: strip_type(arg.arg_type),
                        })
                        .collect(),
                })
                .collect(),
        },
        TypeSpecification::DerivedTypeSpecification {
            annotations,
            type_params,
            base_type,
            from_base_type,
            to_base_type,
        } => TypeSpecification::DerivedTypeSpecification {
            annotations: strip_annotations(annotations),
            type_params,
            base_type: strip_type(base_type),
            from_base_type,
            to_base_type,
        },
    }
}

fn strip_type_definition(node: TypeDefinition) -> TypeDefinition {
    match node {
        TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr: strip_type(type_expr),
        },
        TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors: strip_access_controlled(constructors, |definitions| {
                definitions
                    .into_iter()
                    .map(|constructor| ConstructorDefinition {
                        name: constructor.name,
                        args: constructor
                            .args
                            .into_iter()
                            .map(|arg| ConstructorArg {
                                name: arg.name,
                                arg_type: strip_type(arg.arg_type),
                            })
                            .collect(),
                    })
                    .collect()
            }),
        },
        TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness,
            partial_type_expr,
        } => TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness: strip_incompleteness(incompleteness),
            partial_type_expr: partial_type_expr.map(strip_type),
        },
    }
}

/// A hole's `partialBody` is a type expression, so it carries attributes the testee strips like
/// any other.
fn strip_incompleteness(node: Incompleteness) -> Incompleteness {
    match node {
        Incompleteness::Draft => Incompleteness::Draft,
        Incompleteness::Hole {
            reason,
            partial_body,
        } => Incompleteness::Hole {
            reason,
            partial_body: partial_body.map(strip_type),
        },
    }
}

fn strip_value_specification(node: ValueSpecification) -> ValueSpecification {
    ValueSpecification {
        annotations: strip_annotations(node.annotations),
        inputs: node
            .inputs
            .into_iter()
            .map(|(name, tpe)| (name, strip_type(tpe)))
            .collect(),
        output: strip_type(node.output),
    }
}

fn strip_value_definition(node: ValueDefinition) -> ValueDefinition {
    ValueDefinition {
        input_types: node
            .input_types
            .into_iter()
            .map(|(name, tpe)| (name, strip_type(tpe)))
            .collect(),
        output_type: node.output_type.map(strip_type),
        body: match node.body {
            ValueBody::Expression(value) => ValueBody::Expression(strip_value(value)),
            ValueBody::Native { native_info } => ValueBody::Native { native_info },
            ValueBody::External {
                externals,
                fallback,
            } => ValueBody::External {
                externals,
                fallback: fallback.map(|value| Box::new(strip_value(*value))),
            },
            ValueBody::Incomplete {
                incompleteness,
                partial_body,
            } => ValueBody::Incomplete {
                incompleteness: strip_incompleteness(incompleteness),
                partial_body: partial_body.map(|value| Box::new(strip_value(*value))),
            },
        },
    }
}

fn strip_access_controlled<T>(
    node: AccessControlled<T>,
    strip: impl FnOnce(T) -> T,
) -> AccessControlled<T> {
    AccessControlled {
        access: node.access,
        value: strip(node.value),
    }
}

/// A module manifest's `types` or `values` with every attribute inside it cleared.
///
/// A names-style listing holds nothing but names, so it has nothing to strip; the bodies it names
/// live in the node files beside the manifest and are stripped when those are read.
fn strip_module_entries<D, S>(
    entries: ModuleEntries<D, S>,
    strip_definition: impl Fn(D) -> D,
    strip_specification: impl Fn(S) -> S,
) -> ModuleEntries<D, S> {
    match entries {
        ModuleEntries::Names(names) => ModuleEntries::Names(names),
        ModuleEntries::Definitions(items) => ModuleEntries::Definitions(
            items
                .into_iter()
                .map(|(name, definition)| (name, strip_definition(definition)))
                .collect(),
        ),
        ModuleEntries::Specifications(items) => ModuleEntries::Specifications(
            items
                .into_iter()
                .map(|(name, specification)| (name, strip_specification(specification)))
                .collect(),
        ),
    }
}

fn strip_access_controlled_type_definition(
    node: AccessControlled<Documented<TypeDefinition>>,
) -> AccessControlled<Documented<TypeDefinition>> {
    strip_access_controlled(node, |documented| {
        strip_documented(documented, strip_type_definition)
    })
}

fn strip_access_controlled_value_definition(
    node: AccessControlled<Documented<ValueDefinition>>,
) -> AccessControlled<Documented<ValueDefinition>> {
    strip_access_controlled(node, |documented| {
        strip_documented(documented, strip_value_definition)
    })
}

fn strip_documented<T>(node: Documented<T>, strip: impl FnOnce(T) -> T) -> Documented<T> {
    Documented {
        doc: node.doc,
        value: strip(node.value),
    }
}

fn strip_module_definition(node: ModuleDefinition) -> ModuleDefinition {
    ModuleDefinition {
        types: node
            .types
            .into_iter()
            .map(|(name, definition)| {
                (
                    name,
                    strip_access_controlled(definition, |d| {
                        strip_documented(d, strip_type_definition)
                    }),
                )
            })
            .collect(),
        values: node
            .values
            .into_iter()
            .map(|(name, definition)| {
                (
                    name,
                    strip_access_controlled(definition, |d| {
                        strip_documented(d, strip_value_definition)
                    }),
                )
            })
            .collect(),
        doc: node.doc,
    }
}

fn strip_module_specification(node: ModuleSpecification) -> ModuleSpecification {
    ModuleSpecification {
        annotations: strip_annotations(node.annotations),
        types: node
            .types
            .into_iter()
            .map(|(name, specification)| {
                (
                    name,
                    strip_documented(specification, strip_type_specification),
                )
            })
            .collect(),
        values: node
            .values
            .into_iter()
            .map(|(name, specification)| {
                (
                    name,
                    strip_documented(specification, strip_value_specification),
                )
            })
            .collect(),
        doc: node.doc,
    }
}

fn strip_package_specification(node: PackageSpecification) -> PackageSpecification {
    PackageSpecification {
        modules: node
            .modules
            .into_iter()
            .map(|(name, module)| (name, strip_module_specification(module)))
            .collect(),
    }
}

fn strip_package_definition(node: PackageDefinition) -> PackageDefinition {
    PackageDefinition {
        modules: node
            .modules
            .into_iter()
            .map(|(name, module)| {
                (
                    name,
                    strip_access_controlled(module, strip_module_definition),
                )
            })
            .collect(),
    }
}

fn strip_distribution(node: Distribution) -> Distribution {
    match node {
        Distribution::Library(content) => Distribution::Library(LibraryContent {
            package_name: content.package_name,
            dependencies: strip_dependencies(content.dependencies),
            def: strip_package_definition(content.def),
        }),
        Distribution::Specs(content) => Distribution::Specs(SpecsContent {
            package_name: content.package_name,
            dependencies: strip_dependencies(content.dependencies),
            spec: strip_package_specification(content.spec),
        }),
        Distribution::Application(content) => Distribution::Application(ApplicationContent {
            package_name: content.package_name,
            dependencies: strip_definition_dependencies(content.dependencies),
            def: strip_package_definition(content.def),
            entry_points: content.entry_points,
        }),
    }
}

fn strip_dependencies(
    dependencies: morphir_core::ir::v4::Dependencies,
) -> morphir_core::ir::v4::Dependencies {
    dependencies
        .into_iter()
        .map(|(name, specification)| (name, strip_package_specification(specification)))
        .collect()
}

fn strip_definition_dependencies(
    dependencies: morphir_core::ir::v4::DefinitionDependencies,
) -> morphir_core::ir::v4::DefinitionDependencies {
    dependencies
        .into_iter()
        .map(|(name, definition)| (name, strip_package_definition(definition)))
        .collect()
}
