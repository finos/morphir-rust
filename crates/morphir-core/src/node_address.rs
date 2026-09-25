//! Draft, storage-independent addresses for semantic IR nodes.
//!
//! The authority `ir` deliberately differs from the V4 document-tree `pkg`
//! authority. A node URI identifies a node in a normalized distribution, not
//! the file that happened to contain it.

pub use crate::format_version::ReleaseTriplet as IrFormatVersion;
use crate::ir::v4::serde_v4::with_fingerprint_semantics;
use crate::ir::v4::{TypeEncoding, with_type_encoding};
use crate::naming::{Name, PackageName, Path};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;

mod index;
mod legacy_v3;
pub use index::{IndexedNodeKind, NodeCatalog, NodeIndex, NodeResolutionError, ResolvedNode};
pub use legacy_v3::{LegacyNodeIdError, convert_v3_node_id};

/// A syntactically invalid semantic node URI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NodeUriError {
    /// An unpinned positional path can silently retarget without a guard.
    #[error("an unpinned indexed node address requires a guard")]
    MissingGuard,
    /// The input cannot be interpreted as one canonical node address.
    #[error("invalid node URI: {0}")]
    Invalid(String),
    /// A caller tried to fingerprint a non-positional step or an unserializable node.
    #[error("cannot fingerprint selected semantic child: {0}")]
    InvalidFingerprint(String),
}

/// The artifact selected by the caller's resolution context.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArtifactSelector {
    /// One named package, possibly ambiguous until the context chooses a release.
    Package(PackageName),
    /// A caller-provided current working artifact alias.
    Workspace(String),
}

/// The package containing a module inside a distribution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeOwner {
    OwnPackage,
    Dependency(PackageName),
}

/// The first semantic node selected within a distribution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeRoot {
    Distribution,
    Package,
    Dependency(PackageName),
    EntryPoint(String),
    Module {
        owner: NodeOwner,
        module: Path,
    },
    Type {
        owner: NodeOwner,
        module: Path,
        name: Name,
    },
    Value {
        owner: NodeOwner,
        module: Path,
        name: Name,
    },
}

/// One typed child selection. Variants do not carry irrelevant names or indices.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeStep {
    TypeExpression,
    Body,
    ValueInputType(Name),
    ValueInputAnnotation(Name),
    ValueOutputType,
    DerivedBaseType,
    PartialTypeExpression,
    CustomConstructor(Name),
    ConstructorArgument(usize),
    RecordField(Name),
    ExtensibleRecordField(Name),
    TypeFunctionParameter,
    TypeFunctionResult,
    ReferenceArgument(usize),
    ApplyFunction,
    ApplyArgument,
    FieldSubject,
    DestructurePattern,
    DestructureValue,
    DestructureBody,
    IfCondition,
    IfThen,
    IfElse,
    LambdaPattern,
    LambdaBody,
    LetDefinition(Name),
    LetBody,
    ListElement(usize),
    PatternMatchSubject,
    TupleElement(usize),
    PatternMatchCasePattern(usize),
    PatternMatchCaseBody(usize),
    UpdateSubject,
    UpdateField(Name),
    AsPatternChild,
    PatternTupleElement(usize),
    PatternConstructorArgument(usize),
    HeadTailHead,
    HeadTailTail,
    ExternalFallback,
    IncompletePartialBody,
    HoleExpectedType,
}

impl NodeStep {
    /// Whether this step selects an ordered child whose identity needs a guard.
    pub fn is_positional(&self) -> bool {
        matches!(
            self,
            Self::ConstructorArgument(_)
                | Self::ReferenceArgument(_)
                | Self::ListElement(_)
                | Self::TupleElement(_)
                | Self::PatternMatchCasePattern(_)
                | Self::PatternMatchCaseBody(_)
                | Self::PatternTupleElement(_)
                | Self::PatternConstructorArgument(_)
        )
    }
}

/// Current resolution or a verified immutable artifact snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArtifactRevision {
    Current,
    Pinned(Sha256Digest),
}

/// A lowercase SHA-256 digest in canonical hexadecimal form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Digest the exact bytes of an acquired immutable snapshot.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }

    pub fn parse(value: &str) -> Result<Self, NodeUriError> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(NodeUriError::Invalid("digest must use sha256".into()));
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(NodeUriError::Invalid(
                "digest must have 64 lowercase hex digits".into(),
            ));
        }
        Ok(Self(hex.to_owned()))
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.0)
    }
}

/// Computes one guard from the ordered child selections on a semantic path.
///
/// Call `push` once for each ordered step, from root to leaf. The selected
/// child is a typed IR node, serialized through the normalized model; its
/// source JSON/YAML syntax and document-tree file location never enter the
/// digest. Named ancestors and unrelated siblings are deliberately excluded.
#[derive(Clone, Copy)]
enum FingerprintMode {
    Legacy,
    V4_1,
}

#[derive(Clone)]
pub struct NodeFingerprintBuilder {
    hash: Sha256,
    mode: FingerprintMode,
}

impl Default for NodeFingerprintBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeFingerprintBuilder {
    pub fn new() -> Self {
        let mut hash = Sha256::new();
        hash.update(b"morphir-node-fingerprint-draft.1\0");
        Self {
            hash,
            mode: FingerprintMode::Legacy,
        }
    }

    /// Build a guard for V4.1 nodes, omitting nonsemantic attributes before
    /// the V4 serializer chooses its shorthand or expanded representation.
    pub fn for_v4_1() -> Self {
        Self {
            mode: FingerprintMode::V4_1,
            ..Self::new()
        }
    }

    /// Add one selected ordered child and its typed semantic subtree.
    pub fn push<T: Serialize>(&mut self, step: &NodeStep, child: &T) -> Result<(), NodeUriError> {
        let (role, index) = match step {
            NodeStep::ConstructorArgument(index) => ("constructor/argument", *index),
            NodeStep::ReferenceArgument(index) => ("reference/argument", *index),
            NodeStep::ListElement(index) => ("list/element", *index),
            NodeStep::TupleElement(index) => ("tuple/element", *index),
            NodeStep::PatternMatchCasePattern(index) => ("pattern-match/case/pattern", *index),
            NodeStep::PatternMatchCaseBody(index) => ("pattern-match/case/body", *index),
            NodeStep::PatternTupleElement(index) => ("tuple-pattern/element", *index),
            NodeStep::PatternConstructorArgument(index) => ("constructor-pattern/argument", *index),
            _ => {
                return Err(NodeUriError::InvalidFingerprint(
                    "step is not positional".into(),
                ));
            }
        };
        let mut value = match self.mode {
            FingerprintMode::Legacy => semantic_json(child),
            FingerprintMode::V4_1 => with_fingerprint_semantics(|| semantic_json(child)),
        }
        .map_err(|error| NodeUriError::InvalidFingerprint(error.to_string()))?;
        if matches!(self.mode, FingerprintMode::Legacy) {
            strip_nonsemantic_attributes(&mut value);
        }
        let mut canonical = Vec::new();
        write_canonical_json(&value, &mut canonical)
            .map_err(|error| NodeUriError::InvalidFingerprint(error.to_string()))?;
        self.hash.update((role.len() as u32).to_be_bytes());
        self.hash.update(role.as_bytes());
        self.hash.update((index as u64).to_be_bytes());
        self.hash.update((canonical.len() as u64).to_be_bytes());
        self.hash.update(&canonical);
        Ok(())
    }

    /// Finish and return the lowercase SHA-256 token used in `guard=`.
    pub fn finish(self) -> Sha256Digest {
        Sha256Digest(
            self.hash
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }
}

/// Choose one V4 type spelling even when a caller is serializing a document
/// under a different thread-local profile at the same time.
pub(crate) fn semantic_json<T: Serialize>(
    node: &T,
) -> Result<serde_json::Value, serde_json::Error> {
    with_type_encoding(TypeEncoding::Expanded, || serde_json::to_value(node))
}

// Source coordinates and tool extensions locate or annotate a semantic node;
// they cannot change the identity of a selected ordered child. Keep the
// remaining attributes, including constraints and inferred types.
fn strip_nonsemantic_attributes(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                strip_nonsemantic_attributes(item);
            }
        }
        serde_json::Value::Object(members) => {
            // A document literal contains arbitrary user JSON. Its own
            // `attributes` keys are data, not Morphir node metadata.
            if members.contains_key("DocumentLiteral") {
                return;
            }
            for member in members.values_mut() {
                strip_nonsemantic_attributes(member);
            }
            if let Some(serde_json::Value::Object(attributes)) = members.get_mut("attributes") {
                attributes.remove("source");
                attributes.remove("extensions");
                if attributes.is_empty() {
                    members.remove("attributes");
                }
            }
        }
        _ => {}
    }
}

fn write_canonical_json(
    value: &serde_json::Value,
    output: &mut Vec<u8>,
) -> Result<(), serde_json::Error> {
    use serde_json::Value;
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(flag) => output.extend_from_slice(if *flag { b"true" } else { b"false" }),
        Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
        Value::String(text) => output.extend_from_slice(serde_json::to_string(text)?.as_bytes()),
        Value::Array(items) => {
            output.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_json(item, output)?;
            }
            output.push(b']');
        }
        Value::Object(members) => {
            output.push(b'{');
            let mut keys = members.keys().collect::<Vec<_>>();
            keys.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(serde_json::to_string(key)?.as_bytes());
                output.push(b':');
                write_canonical_json(&members[key], output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

/// A portable address with a canonical Morphir URI spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeUri {
    artifact: ArtifactSelector,
    format: IrFormatVersion,
    root: NodeRoot,
    steps: Vec<NodeStep>,
    revision: ArtifactRevision,
    guard: Option<Sha256Digest>,
}

impl NodeUri {
    /// Construct an address only if its canonical spelling parses back to the
    /// same typed value. This also checks the guard and name invariants.
    pub fn new(
        artifact: ArtifactSelector,
        format: IrFormatVersion,
        root: NodeRoot,
        steps: Vec<NodeStep>,
        revision: ArtifactRevision,
        guard: Option<Sha256Digest>,
    ) -> Result<Self, NodeUriError> {
        let candidate = Self {
            artifact,
            format,
            root,
            steps,
            revision,
            guard,
        };
        let checked = Self::parse(&candidate.to_string())?;
        if checked != candidate {
            return Err(invalid("address is not canonical"));
        }
        Ok(candidate)
    }

    pub fn artifact(&self) -> &ArtifactSelector {
        &self.artifact
    }
    pub fn format(&self) -> IrFormatVersion {
        self.format
    }
    pub fn root(&self) -> &NodeRoot {
        &self.root
    }
    pub fn steps(&self) -> &[NodeStep] {
        &self.steps
    }
    pub fn revision(&self) -> &ArtifactRevision {
        &self.revision
    }
    pub fn guard(&self) -> Option<&Sha256Digest> {
        self.guard.as_ref()
    }

    /// Parse one canonical draft node URI, rejecting aliases and unknown roles.
    pub fn parse(uri: &str) -> Result<Self, NodeUriError> {
        let body = uri
            .strip_prefix("morphir://ir/")
            .ok_or_else(|| invalid("wrong URI authority"))?;
        let (before_fragment, fragment) = body
            .split_once('#')
            .ok_or_else(|| invalid("missing node fragment"))?;
        if fragment.contains('#') {
            return Err(invalid("duplicate fragment"));
        }
        let (selector, query) = before_fragment
            .split_once('?')
            .ok_or_else(|| invalid("missing format query"))?;
        let artifact = if let Some(raw) = selector.strip_prefix("pkg/") {
            if raw.is_empty() {
                return Err(invalid("empty package selector"));
            }
            ArtifactSelector::Package(parse_package(raw)?)
        } else if let Some(alias) = selector.strip_prefix("workspace/") {
            if !valid_workspace_alias(alias) {
                return Err(invalid("invalid workspace alias"));
            }
            ArtifactSelector::Workspace(alias.to_owned())
        } else {
            return Err(invalid("unknown artifact selector"));
        };

        let mut format = None;
        let mut revision = ArtifactRevision::Current;
        let mut guard = None;
        for parameter in query.split('&') {
            let (key, value) = parameter
                .split_once('=')
                .ok_or_else(|| invalid("malformed query parameter"))?;
            match key {
                "format" if format.is_none() => format = Some(parse_format(value)?),
                "rev" if matches!(revision, ArtifactRevision::Current) => {
                    revision = ArtifactRevision::Pinned(Sha256Digest::parse(value)?)
                }
                "guard" if guard.is_none() => guard = Some(Sha256Digest::parse(value)?),
                _ => return Err(invalid("duplicate or unknown query parameter")),
            }
        }
        let format = format.ok_or_else(|| invalid("missing format"))?;
        let parts = fragment
            .strip_prefix('/')
            .ok_or_else(|| invalid("node fragment must start with /"))?
            .split('/')
            .collect::<Vec<_>>();
        let (root, consumed) = parse_root(&parts)?;
        let steps = parse_steps(&parts[consumed..])?;
        if matches!(revision, ArtifactRevision::Current)
            && guard.is_none()
            && steps.iter().any(NodeStep::is_positional)
        {
            return Err(NodeUriError::MissingGuard);
        }
        let address = Self {
            artifact,
            format,
            root,
            steps,
            revision,
            guard,
        };
        if address.to_string() != uri {
            return Err(invalid("noncanonical URI spelling"));
        }
        Ok(address)
    }
}

impl fmt::Display for NodeUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "morphir://ir/")?;
        match &self.artifact {
            ArtifactSelector::Package(package) => {
                write!(f, "pkg/{}", package.to_canonical_string())?
            }
            ArtifactSelector::Workspace(alias) => write!(f, "workspace/{alias}")?,
        }
        write!(f, "?format={}", self.format)?;
        if let ArtifactRevision::Pinned(digest) = &self.revision {
            write!(f, "&rev={digest}")?;
        }
        if let Some(digest) = &self.guard {
            write!(f, "&guard={digest}")?;
        }
        write!(f, "#")?;
        let mut root = Vec::new();
        match &self.root {
            NodeRoot::Distribution => root.push("distribution".to_owned()),
            NodeRoot::Package => root.push("package".to_owned()),
            NodeRoot::Dependency(package) => {
                root.extend([
                    "dependency".to_owned(),
                    encode_component(&package.to_canonical_string()),
                ]);
            }
            NodeRoot::EntryPoint(name) => {
                root.extend(["entry-point".to_owned(), encode_component(name)])
            }
            NodeRoot::Module { owner, module } => {
                push_owner(&mut root, owner);
                root.extend([
                    "module".to_owned(),
                    encode_component(&module.to_canonical_string()),
                ]);
            }
            NodeRoot::Type {
                owner,
                module,
                name,
            }
            | NodeRoot::Value {
                owner,
                module,
                name,
            } => {
                push_owner(&mut root, owner);
                root.extend([
                    "module".to_owned(),
                    encode_component(&module.to_canonical_string()),
                    if matches!(self.root, NodeRoot::Type { .. }) {
                        "type"
                    } else {
                        "value"
                    }
                    .to_owned(),
                    encode_component(&name.to_canonical_string()),
                ]);
            }
        }
        for step in &self.steps {
            match step {
                NodeStep::TypeExpression => root.push("type-exp".to_owned()),
                NodeStep::Body => root.push("body".to_owned()),
                NodeStep::ValueInputType(name) => root.extend([
                    "input".to_owned(),
                    encode_component(&name.to_canonical_string()),
                ]),
                NodeStep::ValueInputAnnotation(name) => root.extend([
                    "input-annotation".to_owned(),
                    encode_component(&name.to_canonical_string()),
                ]),
                NodeStep::ValueOutputType => root.push("output".to_owned()),
                NodeStep::DerivedBaseType => {
                    root.extend(["derived".to_owned(), "base-type".to_owned()])
                }
                NodeStep::PartialTypeExpression => {
                    root.extend(["incomplete".to_owned(), "partial-type".to_owned()])
                }
                NodeStep::CustomConstructor(name) => root.extend([
                    "constructor".to_owned(),
                    encode_component(&name.to_canonical_string()),
                ]),
                NodeStep::ConstructorArgument(index) => {
                    root.extend(["argument".to_owned(), index.to_string()])
                }
                NodeStep::RecordField(name) => root.extend([
                    "record".to_owned(),
                    "field".to_owned(),
                    encode_component(&name.to_canonical_string()),
                ]),
                NodeStep::ExtensibleRecordField(name) => root.extend([
                    "extensible-record".to_owned(),
                    "field".to_owned(),
                    encode_component(&name.to_canonical_string()),
                ]),
                NodeStep::TypeFunctionParameter => {
                    root.extend(["function".to_owned(), "parameter".to_owned()])
                }
                NodeStep::TypeFunctionResult => {
                    root.extend(["function".to_owned(), "result".to_owned()])
                }
                NodeStep::ReferenceArgument(index) => root.extend([
                    "reference".to_owned(),
                    "argument".to_owned(),
                    index.to_string(),
                ]),
                NodeStep::ApplyFunction => root.extend(["apply".to_owned(), "function".to_owned()]),
                NodeStep::ApplyArgument => root.extend(["apply".to_owned(), "argument".to_owned()]),
                NodeStep::FieldSubject => root.extend(["field".to_owned(), "subject".to_owned()]),
                NodeStep::DestructurePattern => {
                    root.extend(["destructure".to_owned(), "pattern".to_owned()])
                }
                NodeStep::DestructureValue => {
                    root.extend(["destructure".to_owned(), "value".to_owned()])
                }
                NodeStep::DestructureBody => {
                    root.extend(["destructure".to_owned(), "body".to_owned()])
                }
                NodeStep::IfCondition => root.extend(["if".to_owned(), "condition".to_owned()]),
                NodeStep::IfThen => root.extend(["if".to_owned(), "then".to_owned()]),
                NodeStep::IfElse => root.extend(["if".to_owned(), "else".to_owned()]),
                NodeStep::LambdaPattern => root.extend(["lambda".to_owned(), "pattern".to_owned()]),
                NodeStep::LambdaBody => root.extend(["lambda".to_owned(), "body".to_owned()]),
                NodeStep::LetDefinition(name) => root.extend([
                    "let".to_owned(),
                    "definition".to_owned(),
                    encode_component(&name.to_canonical_string()),
                ]),
                NodeStep::LetBody => root.extend(["let".to_owned(), "body".to_owned()]),
                NodeStep::ListElement(index) => {
                    root.extend(["list".to_owned(), "element".to_owned(), index.to_string()])
                }
                NodeStep::PatternMatchSubject => {
                    root.extend(["pattern-match".to_owned(), "subject".to_owned()])
                }
                NodeStep::TupleElement(index) => {
                    root.extend(["tuple".to_owned(), "element".to_owned(), index.to_string()])
                }
                NodeStep::PatternMatchCasePattern(index) => root.extend([
                    "pattern-match".to_owned(),
                    "case".to_owned(),
                    index.to_string(),
                    "pattern".to_owned(),
                ]),
                NodeStep::PatternMatchCaseBody(index) => root.extend([
                    "pattern-match".to_owned(),
                    "case".to_owned(),
                    index.to_string(),
                    "body".to_owned(),
                ]),
                NodeStep::UpdateSubject => root.extend(["update".to_owned(), "subject".to_owned()]),
                NodeStep::UpdateField(name) => root.extend([
                    "update".to_owned(),
                    "field".to_owned(),
                    encode_component(&name.to_canonical_string()),
                ]),
                NodeStep::AsPatternChild => {
                    root.extend(["as-pattern".to_owned(), "pattern".to_owned()])
                }
                NodeStep::PatternTupleElement(index) => root.extend([
                    "tuple-pattern".to_owned(),
                    "element".to_owned(),
                    index.to_string(),
                ]),
                NodeStep::PatternConstructorArgument(index) => root.extend([
                    "constructor-pattern".to_owned(),
                    "argument".to_owned(),
                    index.to_string(),
                ]),
                NodeStep::HeadTailHead => root.extend(["head-tail".to_owned(), "head".to_owned()]),
                NodeStep::HeadTailTail => root.extend(["head-tail".to_owned(), "tail".to_owned()]),
                NodeStep::ExternalFallback => {
                    root.extend(["external".to_owned(), "fallback".to_owned()])
                }
                NodeStep::IncompletePartialBody => {
                    root.extend(["incomplete".to_owned(), "partial-body".to_owned()])
                }
                NodeStep::HoleExpectedType => {
                    root.extend(["hole".to_owned(), "expected-type".to_owned()])
                }
            }
        }
        write!(f, "/{}", root.join("/"))
    }
}

fn invalid(message: &str) -> NodeUriError {
    NodeUriError::Invalid(message.to_owned())
}

fn parse_format(value: &str) -> Result<IrFormatVersion, NodeUriError> {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || (part.len() > 1 && part.starts_with('0')))
    {
        return Err(invalid(
            "format must have three canonical numeric components",
        ));
    }
    let parse = |part: &str| {
        part.parse::<u32>()
            .map_err(|_| invalid("invalid format component"))
    };
    Ok(IrFormatVersion::new(
        parse(parts[0])?,
        parse(parts[1])?,
        parse(parts[2])?,
    ))
}

fn parse_package(value: &str) -> Result<PackageName, NodeUriError> {
    let package =
        PackageName::from_canonical_string(value).map_err(|_| invalid("invalid package name"))?;
    if package.is_empty() || package.to_canonical_string() != value {
        return Err(invalid("noncanonical package name"));
    }
    Ok(package)
}

fn parse_path(value: &str) -> Result<Path, NodeUriError> {
    let decoded = decode_component(value)?;
    let path = Path::from_canonical_string(&decoded).map_err(|_| invalid("invalid module path"))?;
    if path.is_empty() || path.to_canonical_string() != decoded {
        return Err(invalid("noncanonical module path"));
    }
    Ok(path)
}

fn parse_name(value: &str) -> Result<Name, NodeUriError> {
    let decoded = decode_component(value)?;
    let name = Name::from_canonical_string(&decoded).map_err(|_| invalid("invalid name"))?;
    if name.to_canonical_string() != decoded {
        return Err(invalid("noncanonical name"));
    }
    Ok(name)
}

fn parse_root(parts: &[&str]) -> Result<(NodeRoot, usize), NodeUriError> {
    match parts {
        ["distribution", ..] => Ok((NodeRoot::Distribution, 1)),
        ["package", ..] => Ok((NodeRoot::Package, 1)),
        ["entry-point", raw, ..] if !raw.is_empty() => {
            Ok((NodeRoot::EntryPoint(decode_component(raw)?), 2))
        }
        _ => {
            let (owner, rest, prefix) = match parts {
                ["dependency", raw, rest @ ..] => (
                    NodeOwner::Dependency(parse_package(&decode_component(raw)?)?),
                    rest,
                    2,
                ),
                other => (NodeOwner::OwnPackage, other, 0),
            };
            if rest.is_empty()
                && prefix == 2
                && let NodeOwner::Dependency(package) = owner
            {
                return Ok((NodeRoot::Dependency(package), 2));
            }
            match rest {
                ["module", module, "type", name, ..] => Ok((
                    NodeRoot::Type {
                        owner,
                        module: parse_path(module)?,
                        name: parse_name(name)?,
                    },
                    prefix + 4,
                )),
                ["module", module, "value", name, ..] => Ok((
                    NodeRoot::Value {
                        owner,
                        module: parse_path(module)?,
                        name: parse_name(name)?,
                    },
                    prefix + 4,
                )),
                ["module", module, ..] => Ok((
                    NodeRoot::Module {
                        owner,
                        module: parse_path(module)?,
                    },
                    prefix + 2,
                )),
                _ => Err(invalid("invalid node root")),
            }
        }
    }
}

fn parse_steps(parts: &[&str]) -> Result<Vec<NodeStep>, NodeUriError> {
    let mut steps = Vec::new();
    let mut cursor = parts;
    while !cursor.is_empty() {
        let (step, count) = match cursor {
            ["type-exp", ..] => (NodeStep::TypeExpression, 1),
            ["body", ..] => (NodeStep::Body, 1),
            ["input", name, ..] => (NodeStep::ValueInputType(parse_name(name)?), 2),
            ["input-annotation", name, ..] => {
                (NodeStep::ValueInputAnnotation(parse_name(name)?), 2)
            }
            ["output", ..] => (NodeStep::ValueOutputType, 1),
            ["derived", "base-type", ..] => (NodeStep::DerivedBaseType, 2),
            ["incomplete", "partial-type", ..] => (NodeStep::PartialTypeExpression, 2),
            ["constructor", name, ..] => (NodeStep::CustomConstructor(parse_name(name)?), 2),
            ["argument", index, ..] => (NodeStep::ConstructorArgument(parse_index(index)?), 2),
            ["record", "field", name, ..] => (NodeStep::RecordField(parse_name(name)?), 3),
            ["extensible-record", "field", name, ..] => {
                (NodeStep::ExtensibleRecordField(parse_name(name)?), 3)
            }
            ["function", "parameter", ..] => (NodeStep::TypeFunctionParameter, 2),
            ["function", "result", ..] => (NodeStep::TypeFunctionResult, 2),
            ["reference", "argument", index, ..] => {
                (NodeStep::ReferenceArgument(parse_index(index)?), 3)
            }
            ["apply", "function", ..] => (NodeStep::ApplyFunction, 2),
            ["apply", "argument", ..] => (NodeStep::ApplyArgument, 2),
            ["field", "subject", ..] => (NodeStep::FieldSubject, 2),
            ["destructure", "pattern", ..] => (NodeStep::DestructurePattern, 2),
            ["destructure", "value", ..] => (NodeStep::DestructureValue, 2),
            ["destructure", "body", ..] => (NodeStep::DestructureBody, 2),
            ["if", "condition", ..] => (NodeStep::IfCondition, 2),
            ["if", "then", ..] => (NodeStep::IfThen, 2),
            ["if", "else", ..] => (NodeStep::IfElse, 2),
            ["lambda", "pattern", ..] => (NodeStep::LambdaPattern, 2),
            ["lambda", "body", ..] => (NodeStep::LambdaBody, 2),
            ["let", "definition", name, ..] => (NodeStep::LetDefinition(parse_name(name)?), 3),
            ["let", "body", ..] => (NodeStep::LetBody, 2),
            ["list", "element", index, ..] => (NodeStep::ListElement(parse_index(index)?), 3),
            ["pattern-match", "subject", ..] => (NodeStep::PatternMatchSubject, 2),
            ["tuple", "element", index, ..] => (NodeStep::TupleElement(parse_index(index)?), 3),
            ["pattern-match", "case", index, "pattern", ..] => {
                (NodeStep::PatternMatchCasePattern(parse_index(index)?), 4)
            }
            ["pattern-match", "case", index, "body", ..] => {
                (NodeStep::PatternMatchCaseBody(parse_index(index)?), 4)
            }
            ["update", "subject", ..] => (NodeStep::UpdateSubject, 2),
            ["update", "field", name, ..] => (NodeStep::UpdateField(parse_name(name)?), 3),
            ["as-pattern", "pattern", ..] => (NodeStep::AsPatternChild, 2),
            ["tuple-pattern", "element", index, ..] => {
                (NodeStep::PatternTupleElement(parse_index(index)?), 3)
            }
            ["constructor-pattern", "argument", index, ..] => {
                (NodeStep::PatternConstructorArgument(parse_index(index)?), 3)
            }
            ["head-tail", "head", ..] => (NodeStep::HeadTailHead, 2),
            ["head-tail", "tail", ..] => (NodeStep::HeadTailTail, 2),
            ["external", "fallback", ..] => (NodeStep::ExternalFallback, 2),
            ["incomplete", "partial-body", ..] => (NodeStep::IncompletePartialBody, 2),
            ["hole", "expected-type", ..] => (NodeStep::HoleExpectedType, 2),
            _ => return Err(invalid("unknown semantic child role")),
        };
        steps.push(step);
        cursor = &cursor[count..];
    }
    Ok(steps)
}

fn parse_index(value: &str) -> Result<usize, NodeUriError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid("invalid positional index"));
    }
    value
        .parse()
        .map_err(|_| invalid("positional index overflow"))
}

fn valid_workspace_alias(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
        && value.as_bytes()[0].is_ascii_lowercase()
}

fn push_owner(parts: &mut Vec<String>, owner: &NodeOwner) {
    if let NodeOwner::Dependency(package) = owner {
        parts.extend([
            "dependency".to_owned(),
            encode_component(&package.to_canonical_string()),
        ]);
    }
}

fn encode_component(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn decode_component(value: &str) -> Result<String, NodeUriError> {
    let mut bytes = Vec::new();
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input
                .next()
                .ok_or_else(|| invalid("truncated percent escape"))?;
            let low = input
                .next()
                .ok_or_else(|| invalid("truncated percent escape"))?;
            let hex = |digit: u8| match digit {
                b'0'..=b'9' => Some(digit - b'0'),
                b'A'..=b'F' => Some(digit - b'A' + 10),
                _ => None,
            };
            bytes.push(
                hex(high)
                    .zip(hex(low))
                    .map(|(high, low)| high * 16 + low)
                    .ok_or_else(|| invalid("invalid percent escape"))?,
            );
        } else {
            if !byte.is_ascii() {
                return Err(invalid("raw non-ASCII URI component"));
            }
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| invalid("invalid UTF-8 escape"))
}
