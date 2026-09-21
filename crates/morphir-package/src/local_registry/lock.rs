use super::diagnostics::resource;
use super::metadata::{reference_shape, release_shape, source_shape};
use super::shape::*;
use super::*;
use crate::resolution::{Binding, IrPackageName, LockedGraph, LockedNode, ReleaseId};
use serde::Serialize;
#[path = "lock_order.rs"]
mod order;
#[path = "lock_structure.rs"]
mod structure;

macro_rules! accessors {($($name:ident:$ty:ty => $doc:literal),*$(,)?)=>{$(#[doc=$doc] pub fn $name(&self)->&$ty{&self.$name})*};}
/// A registry alias and its historical snapshot reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Registry {
    id: LocalId,
    snapshot: LocalId,
}
impl Registry {
    accessors!(id:LocalId=>"Registry alias.",snapshot:LocalId=>"Historical snapshot evidence ID.");
}
/// One release's declared acquisition references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryAcquisition {
    release: ReleaseId,
    registry: LocalId,
    record: ObjectReference,
    source: DirectorySource,
    statement: LocalId,
}
impl LibraryAcquisition {
    accessors!(release:ReleaseId=>"Requested release.",registry:LocalId=>"Registry alias.",record:ObjectReference=>"Registry record reference.",source:DirectorySource=>"Bundle source.",statement:LocalId=>"Publisher statement evidence ID.");
}
/// Recognized historical evidence roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceKind {
    /// Historical root.
    TufRoot,
    /// Historical timestamp.
    TufTimestamp,
    /// Historical snapshot.
    TufSnapshot,
    /// Historical targets.
    TufTargets,
    /// Publisher statement envelope.
    ReleaseStatement,
}
impl EvidenceKind {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "tuf-root" => Some(Self::TufRoot),
            "tuf-timestamp" => Some(Self::TufTimestamp),
            "tuf-snapshot" => Some(Self::TufSnapshot),
            "tuf-targets" => Some(Self::TufTargets),
            "release-statement" => Some(Self::ReleaseStatement),
            _ => None,
        }
    }
    fn role(self) -> &'static str {
        match self {
            Self::TufRoot => "root",
            Self::TufTimestamp => "timestamp",
            Self::TufSnapshot => "snapshot",
            Self::TufTargets => "targets",
            Self::ReleaseStatement => "release-statement",
        }
    }
}
/// A declared evidence object. No signatures have been checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryEvidence {
    id: LocalId,
    registry: LocalId,
    kind: EvidenceKind,
    #[serde(flatten)]
    reference: ObjectReference,
}
impl LibraryEvidence {
    accessors!(id:LocalId=>"Evidence ID.",registry:LocalId=>"Registry alias.",reference:ObjectReference=>"Object reference.");
    /// Recognized evidence role.
    pub fn kind(&self) -> EvidenceKind {
        self.kind
    }
}
/// Decoded lock structure; neither the lock nor its graph establishes authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryLock {
    graph: LockedGraph,
    registries: Vec<Registry>,
    acquisitions: Vec<LibraryAcquisition>,
    evidence: Vec<LibraryEvidence>,
}
impl LibraryLock {
    /// Structurally closed graph, retaining input order.
    pub fn graph(&self) -> &LockedGraph {
        &self.graph
    }
    /// Declared registries.
    pub fn registries(&self) -> &[Registry] {
        &self.registries
    }
    /// Declared release acquisitions.
    pub fn acquisitions(&self) -> &[LibraryAcquisition] {
        &self.acquisitions
    }
    /// Declared historical evidence references.
    pub fn evidence(&self) -> &[LibraryEvidence] {
        &self.evidence
    }
}
/// Decode a bounded draft.3 lock without executing resolution or authentication.
pub fn decode_library_lock(bytes: &[u8]) -> Result<LibraryLock, Diagnostic> {
    let subject = Subject::Lock;
    let parsed = decode_json_domain(bytes, JsonDomain::Lock, &subject, Phase::Decode)?;
    let doc = &parsed.document;
    bounds(doc)?;
    let mut s = Shape::new(&subject);
    let (mut graph, mut registries, mut acquisitions, mut evidence) =
        (None, vec![], vec![], vec![]);
    if top(
        doc,
        &mut s,
        "LibraryLock",
        &[
            "resolution",
            "graph",
            "registries",
            "acquisitions",
            "evidence",
        ],
    ) {
        resolution_shape(doc.get("resolution"), &mut s);
        graph = graph_shape(doc.get("graph"), &mut s);
        registries = s
            .array(doc.get("registries"), "/registries", true)
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let p = format!("/registries/{i}");
                let m = s.closed(Some(v), &p, &["id", "snapshot"])?;
                let id = s.parsed(
                    m.get("id"),
                    &format!("{p}/id"),
                    LocalId::parse,
                    Rule::InvalidName,
                );
                let snapshot = s.parsed(
                    m.get("snapshot"),
                    &format!("{p}/snapshot"),
                    LocalId::parse,
                    Rule::InvalidName,
                );
                Some(Registry {
                    id: id?,
                    snapshot: snapshot?,
                })
            })
            .collect();
        acquisitions = s
            .array(doc.get("acquisitions"), "/acquisitions", true)
            .iter()
            .enumerate()
            .filter_map(|(i, v)| acquisition_shape(v, &mut s, &format!("/acquisitions/{i}")))
            .collect();
        evidence = s
            .array(doc.get("evidence"), "/evidence", true)
            .iter()
            .enumerate()
            .filter_map(|(i, v)| evidence_shape(v, &mut s, &format!("/evidence/{i}")))
            .collect();
    }
    order::order_faults(&mut s.support, doc);
    s.supported(None)?;
    s.finish(Phase::Shape)?;
    let lock = LibraryLock {
        graph: graph.expect("validated graph"),
        registries,
        acquisitions,
        evidence,
    };
    structure::identities(&lock.graph, &mut s);
    if s.violations.is_empty() {
        structure::topology(&lock.graph, &mut s)
    }
    structure::closure(&lock, &mut s);
    s.finish(Phase::Structure)?;
    Ok(lock)
}
fn bounds(doc: &JsonNode) -> Result<(), Diagnostic> {
    let fail = |r, n| resource(&Subject::Lock, Phase::Decode, r, n);
    if let Some(nodes) = member(doc.get("graph"), "nodes").and_then(JsonNode::as_array) {
        if nodes.len() > 512 {
            return Err(fail(Resource::GraphNodes, 512));
        }
        let mut total = 0;
        for node in nodes {
            if let Some(bindings) = node.get("bindings").and_then(JsonNode::as_array) {
                if bindings.len() > 512 {
                    return Err(fail(Resource::NodeBindings, 512));
                }
                total += bindings.len();
                if total > 32768 {
                    return Err(fail(Resource::GraphBindings, 32768));
                }
            }
        }
    }
    if doc
        .get("evidence")
        .and_then(JsonNode::as_array)
        .is_some_and(|a| a.len() > 2048)
    {
        return Err(fail(Resource::EvidenceEntries, 2048));
    }
    Ok(())
}
fn resolution_shape(v: Option<&JsonNode>, s: &mut Shape) {
    let Some(m) = s.closed(
        v,
        "/resolution",
        &["policy", "profile", "requiredCapabilities"],
    ) else {
        return;
    };
    s.literal(
        m.get("policy"),
        "/resolution/policy",
        &["flat-library:0.1.0-draft.2"],
        Code::UnsupportedProfile,
    );
    s.literal(
        m.get("profile"),
        "/resolution/profile",
        &["local-library"],
        Code::UnsupportedProfile,
    );
    let required = ["dsse-ed25519", "local-directory", "tuf-1.0.36"];
    let mut seen = Vec::new();
    for (i, v) in s
        .array(
            m.get("requiredCapabilities"),
            "/resolution/requiredCapabilities",
            false,
        )
        .iter()
        .enumerate()
    {
        let p = format!("/resolution/requiredCapabilities/{i}");
        s.literal(Some(v), &p, &required, Code::UnsupportedCapability);
        // The reference lossless AST gives each number token, array and object
        // its own identity. Only decoded string/bool/null values can repeat.
        if !matches!(
            v,
            JsonNode::Array(_) | JsonNode::Object(_) | JsonNode::Number(_)
        ) && seen.contains(&v)
        {
            s.add(p, Rule::DuplicateIdentity)
        } else {
            seen.push(v)
        }
    }
    if required
        .iter()
        .any(|r| !seen.iter().any(|v| v.as_str() == Some(r)))
    {
        s.add("/resolution/requiredCapabilities", Rule::InvalidValue)
    }
}
fn graph_shape(v: Option<&JsonNode>, s: &mut Shape) -> Option<LockedGraph> {
    let m = s.closed(v, "/graph", &["root", "nodes"])?;
    let root = release_shape(m.get("root"), s, "/graph/root");
    let nodes = s
        .array(m.get("nodes"), "/graph/nodes", true)
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            let p = format!("/graph/nodes/{i}");
            let m = s.closed(
                Some(v),
                &p,
                &[
                    "release",
                    "irPackageName",
                    "manifestDigest",
                    "contentDigest",
                    "bindings",
                ],
            )?;
            let release = release_shape(m.get("release"), s, &format!("{p}/release"));
            let ir_package_name = s.parsed(
                m.get("irPackageName"),
                &format!("{p}/irPackageName"),
                IrPackageName::parse,
                Rule::InvalidName,
            );
            let manifest_digest = s.parsed(
                m.get("manifestDigest"),
                &format!("{p}/manifestDigest"),
                Digest::parse,
                Rule::InvalidDigest,
            );
            let content_digest = s.parsed(
                m.get("contentDigest"),
                &format!("{p}/contentDigest"),
                Digest::parse,
                Rule::InvalidDigest,
            );
            let bindings = s
                .array(m.get("bindings"), &format!("{p}/bindings"), false)
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    let p = format!("{p}/bindings/{i}");
                    let m = s.closed(Some(v), &p, &["irPackageName", "target"])?;
                    let ir_package_name = s.parsed(
                        m.get("irPackageName"),
                        &format!("{p}/irPackageName"),
                        IrPackageName::parse,
                        Rule::InvalidName,
                    );
                    let target = release_shape(m.get("target"), s, &format!("{p}/target"));
                    Some(Binding {
                        ir_package_name: ir_package_name?,
                        target: target?,
                    })
                })
                .collect();
            Some(LockedNode {
                release: release?,
                ir_package_name: ir_package_name?,
                manifest_digest: manifest_digest?,
                content_digest: content_digest?,
                bindings,
            })
        })
        .collect();
    Some(LockedGraph { root: root?, nodes })
}
fn acquisition_shape(v: &JsonNode, s: &mut Shape, p: &str) -> Option<LibraryAcquisition> {
    let m = s.closed(
        Some(v),
        p,
        &["release", "registry", "record", "source", "statement"],
    )?;
    let release = release_shape(m.get("release"), s, &format!("{p}/release"));
    let registry = s.parsed(
        m.get("registry"),
        &format!("{p}/registry"),
        LocalId::parse,
        Rule::InvalidName,
    );
    let statement = s.parsed(
        m.get("statement"),
        &format!("{p}/statement"),
        LocalId::parse,
        Rule::InvalidName,
    );
    let record = reference_shape(
        m.get("record"),
        s,
        &format!("{p}/record"),
        "records/",
        m.get("registry"),
    );
    let source = source_shape(
        m.get("source"),
        s,
        &format!("{p}/source"),
        m.get("registry"),
    );
    Some(LibraryAcquisition {
        release: release?,
        registry: registry?,
        record: record?,
        source: source?,
        statement: statement?,
    })
}
fn evidence_shape(v: &JsonNode, s: &mut Shape, p: &str) -> Option<LibraryEvidence> {
    let m = s.closed(Some(v), p, &["id", "registry", "kind", "path", "digest"])?;
    let id = s.parsed(
        m.get("id"),
        &format!("{p}/id"),
        LocalId::parse,
        Rule::InvalidName,
    );
    let registry = s.parsed(
        m.get("registry"),
        &format!("{p}/registry"),
        LocalId::parse,
        Rule::InvalidName,
    );
    let recognized = s.literal(
        m.get("kind"),
        &format!("{p}/kind"),
        &[
            "tuf-root",
            "tuf-timestamp",
            "tuf-snapshot",
            "tuf-targets",
            "release-statement",
        ],
        Code::UnsupportedProfile,
    );
    let digest = s.parsed(
        m.get("digest"),
        &format!("{p}/digest"),
        Digest::parse,
        Rule::InvalidDigest,
    );
    if !recognized {
        return None;
    }
    let kind = EvidenceKind::parse(m["kind"].as_str().unwrap()).unwrap();
    let path = path(
        m.get("path"),
        s,
        &format!("{p}/path"),
        if kind == EvidenceKind::ReleaseStatement {
            "statements/"
        } else {
            "metadata/"
        },
        m.get("registry"),
    );
    if kind != EvidenceKind::ReleaseStatement {
        s.parsed(
            m.get("path"),
            &format!("{p}/path"),
            |text| {
                let suffix = format!(".{}.json", kind.role());
                let version = text
                    .strip_prefix("metadata/")
                    .and_then(|s| s.strip_suffix(&suffix));
                version
                    .filter(|v| {
                        !v.is_empty()
                            && !v.starts_with('0')
                            && v.bytes().all(|b| b.is_ascii_digit())
                    })
                    .map(|_| ())
                    .ok_or(())
            },
            Rule::InvalidValue,
        );
    }
    Some(LibraryEvidence {
        id: id?,
        registry: registry?,
        kind,
        reference: ObjectReference {
            path: path?,
            digest: digest?,
        },
    })
}
