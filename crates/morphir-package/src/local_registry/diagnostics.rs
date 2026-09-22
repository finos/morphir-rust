use super::{PathRule, Subject, SubjectWire};
use serde::Serialize;

macro_rules! wire_enum {
    ($name:ident, $doc:literal, $($case:ident),+ $(,)?) => {
        #[doc=$doc]
        #[derive(Debug,Clone,Copy,PartialEq,Eq,PartialOrd,Ord,Serialize)]
        #[serde(rename_all="kebab-case")]
        pub enum $name { $(#[doc=stringify!($case)] $case),+ }
    }
}
wire_enum!(
    Phase,
    "Ordered validation phases.",
    Decode,
    Support,
    Shape,
    Structure,
    Repository,
    Authorization,
    Catalog,
    Bundle,
    Graph,
    Publication,
    Commit
);
wire_enum!(
    Code,
    "Closed local-registry rejection codes.",
    UnsafePath,
    InvalidInput,
    ResourceLimit,
    UnsupportedProfile,
    UnsupportedSource,
    UnsupportedCapability,
    UnsupportedPayloadType,
    CapabilityUnavailable,
    UnauthorizedPublisher,
    SignatureInvalid
);
wire_enum!(
    Resource,
    "Bounded profile resources.",
    LockBytes,
    PolicyBytes,
    RecordBytes,
    StatementBytes,
    EnvelopeBytes,
    RootBytes,
    TimestampBytes,
    SnapshotBytes,
    TargetsBytes,
    JsonDepth,
    GraphNodes,
    NodeBindings,
    GraphBindings,
    EvidenceEntries,
    PublisherRules,
    NamespaceGrants,
    PublisherKeys,
    Signatures,
    PathBytes,
    PathComponents,
    ComponentBytes
);
wire_enum!(
    Rule,
    "Structural violation rules.",
    MalformedJson,
    DuplicateKey,
    InvalidType,
    MissingField,
    UnknownField,
    InvalidValue,
    InvalidName,
    InvalidVersion,
    InvalidDigest,
    InvalidInterval,
    DuplicateIdentity,
    IdentityMismatch,
    MissingRoot,
    UnreachableNode,
    DanglingBinding,
    Cycle,
    UnsupportedFlatBinding,
    Noncanonical,
    MissingReference,
    OrphanReference,
    EvidenceKindMismatch,
    TargetLinkMismatch,
    DependencyMismatch,
    ThresholdExceedsKeys
);
wire_enum!(
    Category,
    "Diagnostic category.",
    InvalidInput,
    UnsupportedCapability,
    DomainRejection
);
/// One closed, serializable diagnostic witness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Witness {
    /// Unsafe path spelling.
    Path {
        subject: SubjectWire,
        rule: PathRule,
    },
    /// Structural violation at a JSON pointer.
    Violation {
        subject: SubjectWire,
        pointer: String,
        rule: Rule,
    },
    /// Unsupported literal.
    Unsupported {
        subject: SubjectWire,
        pointer: String,
        value: String,
    },
    /// Fatal resource limit, with capped observed count.
    Resource {
        subject: SubjectWire,
        resource: Resource,
        scope: ProfileScope,
        maximum: String,
        observed: String,
    },
    /// No publisher rule authorizes the requested release.
    Authority {
        registry: String,
        release: crate::resolution::ReleaseId,
        rule: AuthorityRule,
    },
    /// Publisher verification did not meet the selected threshold.
    Authentication {
        subject: SubjectWire,
        role: PublisherRole,
        required: String,
        verified: String,
    },
}
wire_enum!(ProfileScope, "Resource scope.", Profile);
wire_enum!(
    AuthorityRule,
    "Publisher authority rejection.",
    PublisherRuleMissing
);
wire_enum!(PublisherRole, "Authentication role.", Publisher);
/// A rejected interpretation or publisher verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("local-registry rejection: {code:?} during {phase:?}")]
pub struct Diagnostic {
    /// Failure category.
    pub category: Category,
    /// Stable rejection code.
    pub code: Code,
    /// Earliest failing phase.
    pub phase: Phase,
    /// Canonically ordered evidence, or the first fatal witness.
    pub witnesses: Vec<Witness>,
}
/// A candidate fault before deterministic selection.
#[derive(Debug, Clone)]
pub struct Fault {
    /// Validation phase.
    pub phase: Phase,
    /// Rejection code.
    pub code: Code,
    /// Logical-object ordered witnesses.
    pub witnesses: Vec<Witness>,
}
fn rank(code: Code) -> u8 {
    match code {
        Code::ResourceLimit => 0,
        Code::UnsafePath => 1,
        Code::InvalidInput => 2,
        _ => 3,
    }
}
/// Select the first phase, then resource/path/invalid/support priority and code.
pub fn select_failure(faults: &[Fault]) -> Option<Diagnostic> {
    let selected = faults.iter().min_by_key(|f| {
        (
            f.phase,
            rank(f.code),
            serde_json::to_string(&f.code).unwrap(),
        )
    })?;
    let mut witnesses: Vec<_> = faults
        .iter()
        .filter(|f| f.phase == selected.phase && f.code == selected.code)
        .flat_map(|f| f.witnesses.clone())
        .collect();
    if matches!(selected.code, Code::ResourceLimit | Code::UnsafePath) {
        witnesses.truncate(1);
    } else {
        witnesses.sort_by_cached_key(|w| canonical_diagnostic(&serde_json::to_value(w).unwrap()));
        witnesses.dedup();
    }
    Some(Diagnostic {
        category: match selected.code {
            Code::InvalidInput => Category::InvalidInput,
            Code::ResourceLimit
            | Code::UnsafePath
            | Code::UnauthorizedPublisher
            | Code::SignatureInvalid => Category::DomainRejection,
            _ => Category::UnsupportedCapability,
        },
        code: selected.code,
        phase: selected.phase,
        witnesses,
    })
}
/// Canonical UTF-8 key ordering for diagnostic comparison.
pub fn canonical_diagnostic(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(m) => {
            let mut entries: Vec<_> = m.iter().collect();
            entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(k, v)| format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical_diagnostic(v)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        serde_json::Value::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(canonical_diagnostic)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}
pub(crate) fn invalid(
    subject: &Subject,
    phase: Phase,
    violations: Vec<(String, Rule)>,
) -> Diagnostic {
    select_failure(&[Fault {
        phase,
        code: Code::InvalidInput,
        witnesses: violations
            .into_iter()
            .map(|(pointer, rule)| Witness::Violation {
                subject: subject.into(),
                pointer,
                rule,
            })
            .collect(),
    }])
    .unwrap()
}
pub(crate) fn resource(
    subject: &Subject,
    phase: Phase,
    res: Resource,
    maximum: usize,
) -> Diagnostic {
    Diagnostic {
        category: Category::DomainRejection,
        code: Code::ResourceLimit,
        phase,
        witnesses: vec![Witness::Resource {
            subject: subject.into(),
            resource: res,
            scope: ProfileScope::Profile,
            maximum: maximum.to_string(),
            observed: (maximum + 1).to_string(),
        }],
    }
}
