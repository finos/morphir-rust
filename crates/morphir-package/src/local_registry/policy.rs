use super::diagnostics::resource;
use super::shape::*;
use super::*;
use crate::resolution::PackagePath;
use serde::Serialize;
use std::collections::HashSet;
/// A selected namespace publisher rule from trusted local configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublisherRule {
    namespace: Namespace,
    public_keys: Vec<PublisherKey>,
    threshold: u64,
}
impl PublisherRule {
    /// Authorized namespace.
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }
    /// Distinct authorized Ed25519 public keys.
    pub fn public_keys(&self) -> &[PublisherKey] {
        &self.public_keys
    }
    /// Positive achievable signature threshold.
    pub fn threshold(&self) -> u64 {
        self.threshold
    }
}
/// A trusted bootstrap root pinned by version and digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BootstrapRoot {
    version: u64,
    digest: Digest,
}
impl BootstrapRoot {
    /// Positive safe-integer root version.
    pub fn version(&self) -> u64 {
        self.version
    }
    /// Pinned root digest.
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}
/// A repository identity and locally authorized namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRepository {
    identity: Digest,
    bootstrap_root: BootstrapRoot,
    namespaces: Vec<Namespace>,
}
impl PolicyRepository {
    /// Pinned repository identity.
    pub fn identity(&self) -> &Digest {
        &self.identity
    }
    /// Bootstrap root pin.
    pub fn bootstrap_root(&self) -> &BootstrapRoot {
        &self.bootstrap_root
    }
    /// Namespace grants.
    pub fn namespaces(&self) -> &[Namespace] {
        &self.namespaces
    }
}
/// Policy governing use after an earlier authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContinuedUse {
    /// Earlier authorization may be reused by a later restore implementation.
    PreviousAuthorization,
    /// Require fresh repository metadata.
    FreshMetadata,
}
/// Decoded trusted local policy. Registry bytes cannot provision this trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustPolicy {
    repositories: Vec<PolicyRepository>,
    publisher_rules: Vec<PublisherRule>,
    continued_use: ContinuedUse,
}
impl TrustPolicy {
    /// Authorized repositories.
    pub fn repositories(&self) -> &[PolicyRepository] {
        &self.repositories
    }
    /// Publisher namespace rules.
    pub fn publisher_rules(&self) -> &[PublisherRule] {
        &self.publisher_rules
    }
    /// Continued-use policy.
    pub fn continued_use(&self) -> ContinuedUse {
        self.continued_use
    }
}
/// Decode policy supplied by trusted local configuration, never by a registry.
///
/// ```
/// use morphir_package::local_registry::{decode_trust_policy, matching_publisher_rule};
/// use morphir_package::resolution::PackagePath;
/// let policy = decode_trust_policy(br#"{
///   "formatVersion":"0.1.0-draft.3", "kind":"LibraryTrustPolicy",
///   "repositories":[], "publisherRules":[], "continuedUse":"fresh-metadata"
/// }"#)?;
/// let path = PackagePath::parse("example.com/finance/a").unwrap();
/// assert!(matching_publisher_rule(&policy, &path).is_none());
/// # Ok::<(), morphir_package::local_registry::Diagnostic>(())
/// ```
pub fn decode_trust_policy(bytes: &[u8]) -> Result<TrustPolicy, Diagnostic> {
    let subject = Subject::Policy;
    let parsed = decode_json_domain(bytes, JsonDomain::Policy, &subject, Phase::Decode)?;
    let doc = &parsed.document;
    bounds(doc)?;
    let mut s = Shape::new(&subject);
    let (mut repositories, mut publisher_rules, mut continued_use) = (vec![], vec![], None);
    if top(
        doc,
        &mut s,
        "LibraryTrustPolicy",
        &["repositories", "publisherRules", "continuedUse"],
    ) {
        if s.literal(
            doc.get("continuedUse"),
            "/continuedUse",
            &["previous-authorization", "fresh-metadata"],
            Code::UnsupportedProfile,
        ) {
            continued_use = Some(
                if doc.get("continuedUse").and_then(JsonNode::as_str)
                    == Some("previous-authorization")
                {
                    ContinuedUse::PreviousAuthorization
                } else {
                    ContinuedUse::FreshMetadata
                },
            )
        }
        repositories = s
            .array(doc.get("repositories"), "/repositories", false)
            .iter()
            .enumerate()
            .filter_map(|(i, v)| repository(v, &mut s, &format!("/repositories/{i}")))
            .collect();
        publisher_rules = s
            .array(doc.get("publisherRules"), "/publisherRules", false)
            .iter()
            .enumerate()
            .filter_map(|(i, v)| publisher(v, &mut s, &format!("/publisherRules/{i}")))
            .collect();
    }
    s.supported(None)?;
    s.finish(Phase::Shape)?;
    unique(
        repositories
            .iter()
            .enumerate()
            .map(|(i, r)| (format!("/repositories/{i}/identity"), r.identity.as_str())),
        &mut s,
    );
    unique(
        publisher_rules.iter().enumerate().map(|(i, r)| {
            (
                format!("/publisherRules/{i}/namespace"),
                r.namespace.as_str(),
            )
        }),
        &mut s,
    );
    for (i, r) in repositories.iter().enumerate() {
        unique(
            r.namespaces
                .iter()
                .enumerate()
                .map(|(j, n)| (format!("/repositories/{i}/namespaces/{j}"), n.as_str())),
            &mut s,
        )
    }
    for (i, r) in publisher_rules.iter().enumerate() {
        unique(
            r.public_keys
                .iter()
                .enumerate()
                .map(|(j, k)| (format!("/publisherRules/{i}/publicKeys/{j}"), k.as_str())),
            &mut s,
        );
        if r.threshold > r.public_keys.iter().collect::<HashSet<_>>().len() as u64 {
            s.add(
                format!("/publisherRules/{i}/threshold"),
                Rule::ThresholdExceedsKeys,
            )
        }
    }
    s.finish(Phase::Structure)?;
    Ok(TrustPolicy {
        repositories,
        publisher_rules,
        continued_use: continued_use.expect("supported policy"),
    })
}
fn bounds(doc: &JsonNode) -> Result<(), Diagnostic> {
    let fail = |res, n| resource(&Subject::Policy, Phase::Decode, res, n);
    if let Some(rules) = doc.get("publisherRules").and_then(JsonNode::as_array) {
        if rules.len() > 1024 {
            return Err(fail(Resource::PublisherRules, 1024));
        }
        for rule in rules {
            if rule
                .get("publicKeys")
                .and_then(JsonNode::as_array)
                .is_some_and(|a| a.len() > 64)
            {
                return Err(fail(Resource::PublisherKeys, 64));
            }
        }
    }
    let mut grants = 0;
    for r in doc
        .get("repositories")
        .and_then(JsonNode::as_array)
        .unwrap_or_default()
    {
        grants += r
            .get("namespaces")
            .and_then(JsonNode::as_array)
            .map_or(0, |a| a.len());
        if grants > 1024 {
            return Err(fail(Resource::NamespaceGrants, 1024));
        }
    }
    Ok(())
}
fn repository(v: &JsonNode, s: &mut Shape, p: &str) -> Option<PolicyRepository> {
    let m = s.closed(Some(v), p, &["identity", "bootstrapRoot", "namespaces"])?;
    let identity = s.parsed(
        m.get("identity"),
        &format!("{p}/identity"),
        Digest::parse,
        Rule::InvalidDigest,
    );
    let mut bootstrap_root = None;
    if let Some(root) = s.closed(
        m.get("bootstrapRoot"),
        &format!("{p}/bootstrapRoot"),
        &["version", "digest"],
    ) {
        let version = s.positive(root.get("version"), &format!("{p}/bootstrapRoot/version"));
        let digest = s.parsed(
            root.get("digest"),
            &format!("{p}/bootstrapRoot/digest"),
            Digest::parse,
            Rule::InvalidDigest,
        );
        if let (Some(version), Some(digest)) = (version, digest) {
            bootstrap_root = Some(BootstrapRoot { version, digest })
        }
    }
    let namespaces = s
        .array(m.get("namespaces"), &format!("{p}/namespaces"), true)
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            s.parsed(
                Some(v),
                &format!("{p}/namespaces/{i}"),
                Namespace::parse,
                Rule::InvalidName,
            )
        })
        .collect();
    Some(PolicyRepository {
        identity: identity?,
        bootstrap_root: bootstrap_root?,
        namespaces,
    })
}
fn publisher(v: &JsonNode, s: &mut Shape, p: &str) -> Option<PublisherRule> {
    let m = s.closed(Some(v), p, &["namespace", "publicKeys", "threshold"])?;
    let namespace = s.parsed(
        m.get("namespace"),
        &format!("{p}/namespace"),
        Namespace::parse,
        Rule::InvalidName,
    );
    let threshold = s.positive(m.get("threshold"), &format!("{p}/threshold"));
    let public_keys = s
        .array(m.get("publicKeys"), &format!("{p}/publicKeys"), true)
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            s.parsed(
                Some(v),
                &format!("{p}/publicKeys/{i}"),
                PublisherKey::parse,
                Rule::InvalidValue,
            )
        })
        .collect();
    Some(PublisherRule {
        namespace: namespace?,
        public_keys,
        threshold: threshold?,
    })
}
fn namespace_matches(namespace: &Namespace, path: &PackagePath) -> bool {
    let mut target = path.as_str().split('/');
    namespace
        .as_str()
        .split('/')
        .all(|n| target.next() == Some(n))
}
/// Whether a pinned repository grants the package's namespace.
pub fn repository_permits(policy: &TrustPolicy, identity: &Digest, path: &PackagePath) -> bool {
    policy
        .repositories
        .iter()
        .any(|r| &r.identity == identity && r.namespaces.iter().any(|n| namespace_matches(n, path)))
}
/// Select the longest component-matching publisher rule, without fallback.
pub fn matching_publisher_rule<'a>(
    policy: &'a TrustPolicy,
    path: &PackagePath,
) -> Option<&'a PublisherRule> {
    policy
        .publisher_rules
        .iter()
        .filter(|r| namespace_matches(&r.namespace, path))
        .max_by_key(|r| r.namespace.as_str().split('/').count())
}
/// Count distinct already-verified authorized keys. This performs no cryptography.
pub fn publisher_threshold_met(rule: Option<&PublisherRule>, keys: &[PublisherKey]) -> bool {
    rule.is_some_and(|r| {
        keys.iter()
            .filter(|k| r.public_keys.contains(k))
            .collect::<HashSet<_>>()
            .len() as u64
            >= r.threshold
    })
}
