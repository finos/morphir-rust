use super::diagnostics::{invalid, resource};
use super::shape::*;
use super::*;
use crate::resolution::{
    IrPackageName, PackagePath, ReleaseId, ReleaseRecord, Requirement, StableVersion, VersionRange,
};
use serde::Serialize;
/// A validated relative object reference, not authenticated content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectReference {
    pub(crate) path: RegistryPath,
    pub(crate) digest: Digest,
}
impl ObjectReference {
    /// Portable registry-relative path.
    pub fn path(&self) -> &RegistryPath {
        &self.path
    }
    /// Declared content digest.
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}
/// The supported local directory source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DirectorySource {
    /// A directory beneath the registry bundles root.
    RegistryDirectory { path: RegistryPath },
}
impl DirectorySource {
    /// Portable directory path.
    pub fn path(&self) -> &RegistryPath {
        match self {
            Self::RegistryDirectory { path } => path,
        }
    }
}
/// Canonical registry metadata decoded independently of authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegistryRecord {
    #[serde(flatten)]
    release: ReleaseRecord,
    source: DirectorySource,
    statement: ObjectReference,
}
impl RegistryRecord {
    /// The release metadata shared with resolution.
    pub fn release_record(&self) -> &ReleaseRecord {
        &self.release
    }
    /// Declared bundle directory.
    pub fn source(&self) -> &DirectorySource {
        &self.source
    }
    /// Declared publisher statement object.
    pub fn statement(&self) -> &ObjectReference {
        &self.statement
    }
}
pub(crate) fn release_shape(v: Option<&JsonNode>, s: &mut Shape, p: &str) -> Option<ReleaseId> {
    let m = s.closed(v, p, &["packagePath", "version"])?;
    let package_path = s.parsed(
        m.get("packagePath"),
        &format!("{p}/packagePath"),
        PackagePath::parse,
        Rule::InvalidName,
    );
    let version = s.parsed(
        m.get("version"),
        &format!("{p}/version"),
        StableVersion::parse,
        Rule::InvalidVersion,
    );
    Some(ReleaseId {
        package_path: package_path?,
        version: version?,
    })
}
pub(crate) fn reference_shape(
    v: Option<&JsonNode>,
    s: &mut Shape,
    p: &str,
    prefix: &str,
    registry: Option<&JsonNode>,
) -> Option<ObjectReference> {
    let m = s.closed(v, p, &["path", "digest"])?;
    let path = path(m.get("path"), s, &format!("{p}/path"), prefix, registry);
    let digest = s.parsed(
        m.get("digest"),
        &format!("{p}/digest"),
        Digest::parse,
        Rule::InvalidDigest,
    );
    Some(ObjectReference {
        path: path?,
        digest: digest?,
    })
}
pub(crate) fn source_shape(
    v: Option<&JsonNode>,
    s: &mut Shape,
    p: &str,
    registry: Option<&JsonNode>,
) -> Option<DirectorySource> {
    let known = member(v, "kind").and_then(JsonNode::as_str) == Some("registry-directory");
    let m = s.object(
        v,
        p,
        if known { &["kind", "path"] } else { &["kind"] },
        &["kind", "path"],
    )?;
    if s.literal(
        m.get("kind"),
        &format!("{p}/kind"),
        &["registry-directory"],
        Code::UnsupportedSource,
    ) {
        path(m.get("path"), s, &format!("{p}/path"), "bundles/", registry)
            .map(|path| DirectorySource::RegistryDirectory { path })
    } else {
        None
    }
}
fn requirement(v: &JsonNode, s: &mut Shape, p: &str) -> Option<Requirement> {
    let m = s.closed(
        Some(v),
        p,
        &["irPackageName", "packagePath", "versionRange"],
    )?;
    let ir_package_name = s.parsed(
        m.get("irPackageName"),
        &format!("{p}/irPackageName"),
        IrPackageName::parse,
        Rule::InvalidName,
    );
    let package_path = s.parsed(
        m.get("packagePath"),
        &format!("{p}/packagePath"),
        PackagePath::parse,
        Rule::InvalidName,
    );
    let mut range = None;
    if let Some(r) = s.closed(
        m.get("versionRange"),
        &format!("{p}/versionRange"),
        &["minimumInclusive", "maximumExclusive"],
    ) {
        let minimum_inclusive = s.parsed(
            r.get("minimumInclusive"),
            &format!("{p}/versionRange/minimumInclusive"),
            StableVersion::parse,
            Rule::InvalidVersion,
        );
        let maximum_exclusive = s.parsed(
            r.get("maximumExclusive"),
            &format!("{p}/versionRange/maximumExclusive"),
            StableVersion::parse,
            Rule::InvalidVersion,
        );
        if let (Some(minimum_inclusive), Some(maximum_exclusive)) =
            (minimum_inclusive, maximum_exclusive)
        {
            range = Some(VersionRange {
                minimum_inclusive,
                maximum_exclusive,
            })
        }
    }
    Some(Requirement {
        ir_package_name: ir_package_name?,
        package_path: package_path?,
        version_range: range?,
    })
}
/// Decode a canonical record, requiring exactly one final LF.
pub fn decode_registry_record(
    bytes: &[u8],
    subject: &Subject,
) -> Result<RegistryRecord, Diagnostic> {
    let (release, extra) = decode_record(bytes, subject, true)?;
    let (source, statement) = extra.expect("validated record members");
    Ok(RegistryRecord {
        release,
        source,
        statement,
    })
}
/// Decode a canonical release statement payload, requiring no final LF.
pub fn decode_release_statement(
    bytes: &[u8],
    subject: &Subject,
) -> Result<ReleaseRecord, Diagnostic> {
    decode_record(bytes, subject, false).map(|(release, _)| release)
}
type RecordMembers = Option<(DirectorySource, ObjectReference)>;
fn decode_record(
    bytes: &[u8],
    subject: &Subject,
    record: bool,
) -> Result<(ReleaseRecord, RecordMembers), Diagnostic> {
    let phase = Phase::Repository;
    let parsed = decode_json_domain(
        bytes,
        if record {
            JsonDomain::Record
        } else {
            JsonDomain::Statement
        },
        subject,
        phase,
    )?;
    let doc = &parsed.document;
    if doc
        .get("dependencies")
        .and_then(JsonNode::as_array)
        .is_some_and(|a| a.len() > 512)
    {
        return Err(resource(subject, phase, Resource::NodeBindings, 512));
    }
    let mut s = Shape::new(subject);
    let mut result = None;
    let mut extra = None;
    let mut names = vec![
        "release",
        "irPackageName",
        "dependencies",
        "manifestDigest",
        "contentDigest",
    ];
    if record {
        names.extend(["source", "statement"])
    }
    if top(
        doc,
        &mut s,
        if record {
            "LibraryRegistryRecord"
        } else {
            "LibraryReleaseStatement"
        },
        &names,
    ) {
        let release = release_shape(doc.get("release"), &mut s, "/release");
        let ir_package_name = s.parsed(
            doc.get("irPackageName"),
            "/irPackageName",
            IrPackageName::parse,
            Rule::InvalidName,
        );
        let manifest_digest = s.parsed(
            doc.get("manifestDigest"),
            "/manifestDigest",
            Digest::parse,
            Rule::InvalidDigest,
        );
        let content_digest = s.parsed(
            doc.get("contentDigest"),
            "/contentDigest",
            Digest::parse,
            Rule::InvalidDigest,
        );
        let dependencies = s
            .array(doc.get("dependencies"), "/dependencies", false)
            .iter()
            .enumerate()
            .filter_map(|(i, v)| requirement(v, &mut s, &format!("/dependencies/{i}")))
            .collect();
        if record {
            let source = source_shape(doc.get("source"), &mut s, "/source", None);
            let statement = reference_shape(
                doc.get("statement"),
                &mut s,
                "/statement",
                "statements/",
                None,
            );
            if let (Some(source), Some(statement)) = (source, statement) {
                extra = Some((source, statement))
            }
        }
        if let (Some(release), Some(ir_package_name), Some(manifest_digest), Some(content_digest)) =
            (release, ir_package_name, manifest_digest, content_digest)
        {
            result = Some(ReleaseRecord {
                release,
                ir_package_name,
                manifest_digest,
                content_digest,
                dependencies,
            })
        }
    }
    s.supported(Some(phase))?;
    s.finish(phase)?;
    let result = result.expect("validated release members");
    let sorted = result
        .dependencies()
        .windows(2)
        .all(|a| a[0].ir_package_name().as_str() <= a[1].ir_package_name().as_str());
    if !sorted
        || parsed.text
            != format!(
                "{}{}",
                canonical_metadata(doc),
                if record { "\n" } else { "" }
            )
    {
        return Err(invalid(
            subject,
            phase,
            vec![("".into(), Rule::Noncanonical)],
        ));
    }
    unique(
        result.dependencies().iter().enumerate().map(|(i, d)| {
            (
                format!("/dependencies/{i}/irPackageName"),
                d.ir_package_name().as_str(),
            )
        }),
        &mut s,
    );
    for (i, d) in result.dependencies().iter().enumerate() {
        if d.version_range.minimum_inclusive >= d.version_range.maximum_exclusive {
            s.add(
                format!("/dependencies/{i}/versionRange"),
                Rule::InvalidInterval,
            )
        }
    }
    s.finish(phase)?;
    Ok((result, extra))
}
fn canonical_metadata(v: &JsonNode) -> String {
    match v {
        JsonNode::String(s) => serde_json::to_string(s).unwrap(),
        JsonNode::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(canonical_metadata)
                .collect::<Vec<_>>()
                .join(",")
        ),
        JsonNode::Object(m) => {
            let mut entries: Vec<_> = m.iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(k, v)| format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical_metadata(v)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        _ => unreachable!("metadata shape contains only strings, arrays and objects"),
    }
}
