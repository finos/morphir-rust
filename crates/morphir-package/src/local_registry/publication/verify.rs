use super::{Error, digest};
use crate::{
    authoring::AuthoredLibrary,
    local_registry::{
        tuf::{self, ProfileMetadata},
        *,
    },
};
use serde_json::{Value, json};

pub(super) fn policy(bytes: &[u8]) -> Result<TrustPolicy, Error> {
    let policy = decode_trust_policy(bytes)?;
    if policy.repositories().len() != 1 {
        return Err(Error::Invalid("one explicitly trusted repository required"));
    }
    Ok(policy)
}
pub(super) fn root(
    policy: &TrustPolicy,
    bytes: &[u8],
    now: jiff::Timestamp,
) -> Result<ProfileMetadata, Error> {
    let roots = tuf::AuthenticatedRoots::from_policy(&policy.repositories()[0], &[bytes.to_vec()])?;
    let root = roots.current().clone();
    fresh(&root, now)?;
    Ok(root)
}
pub(super) fn fresh(metadata: &ProfileMetadata, now: jiff::Timestamp) -> Result<(), Error> {
    let expires = metadata.document()["signed"]["expires"]
        .as_str()
        .ok_or(Error::Invalid("expiry"))?
        .parse::<jiff::Timestamp>()
        .map_err(|_| Error::Invalid("expiry"))?;
    if expires <= now {
        return Err(Error::Invalid("metadata-expired"));
    }
    Ok(())
}
pub(super) fn role(
    root: &ProfileMetadata,
    bytes: &[u8],
    kind: TufRole,
    now: jiff::Timestamp,
) -> Result<ProfileMetadata, Error> {
    let metadata = tuf::decode_profile(bytes, kind)?;
    tuf::verify_quorum(root, &metadata)?;
    fresh(&metadata, now)?;
    Ok(metadata)
}
pub(super) fn version(metadata: &ProfileMetadata) -> package_tough::schema::Version {
    metadata.document()["signed"]["version"]
        .to_string()
        .parse()
        .expect("profile checked positive integer")
}
pub(super) fn release(
    library: &AuthoredLibrary,
    record: &[u8],
    envelope: &[u8],
    policy: &TrustPolicy,
) -> Result<RegistryRecord, Error> {
    let record_bytes = record;
    let subject = ObjectSubject {
        registry: LocalId::parse("local").expect("literal"),
        path: RegistryPath::parse("records/request.json").expect("literal"),
    };
    let record = decode_registry_record(record, &Subject::from(&subject))?;
    let release = record.release_record();
    let requested = release.release();
    let manifest = library.metadata().value();
    if requested.package_path().as_str() != manifest["packagePath"].as_str().unwrap_or("")
        || requested.version().as_str() != manifest["version"].as_str().unwrap_or("")
        || release.ir_package_name().as_str()
            != manifest["ir"]["packageName"].as_str().unwrap_or("")
        || !release.dependencies().is_empty()
        || release.manifest_digest().as_str() != library.metadata().manifest_digest().to_string()
        || release.content_digest().as_str() != library.metadata().content_digest().to_string()
        || record.source().path().as_str()
            != format!(
                "bundles/{}",
                &library.metadata().content_digest().to_string()[7..]
            )
        || record.statement().digest().as_str() != digest(envelope)
    {
        return Err(Error::Invalid("cross-document-mismatch"));
    }
    if !repository_permits(
        policy,
        policy.repositories()[0].identity(),
        requested.package_path(),
    ) {
        return Err(Error::Invalid("namespace-denied"));
    }
    let prepared = prepare_publisher_envelope(envelope, &subject)?;
    let evidence = verify_publisher_signatures(&prepared, requested, policy)?;
    let statement = decode_release_statement(evidence.payload_bytes(), &Subject::from(&subject))?;
    if &statement != release {
        return Err(Error::Invalid("cross-document-mismatch"));
    }
    physical(&record_path(&record), record_bytes)?;
    physical(record.statement().path().as_str(), envelope)?;
    Ok(record)
}
pub(super) fn pin(bytes: &[u8]) -> Value {
    json!({"length":bytes.len(),"hashes":{"sha256":&digest(bytes)[7..]}})
}
pub(super) fn record_path(record: &RegistryRecord) -> String {
    format!(
        "records/{}/release.json",
        &record.release_record().content_digest().as_str()[7..]
    )
}
pub(super) fn targets(record: &RegistryRecord, record_bytes: &[u8], envelope: &[u8]) -> Value {
    let mut pin = pin(record_bytes);
    pin["custom"] = json!({"morphir":{"formatVersion":"0.1.0-draft.3","kind":"LibraryRelease",
        "release":record.release_record().release(),"status":"active"}});
    let mut statement = self::pin(envelope);
    statement["custom"] = json!({"morphir": {"formatVersion": "0.1.0-draft.3",
        "kind": "LibraryReleaseStatement", "release": record.release_record().release()}});
    json!({record_path(record):pin,record.statement().path().as_str():statement})
}
pub(super) fn physical(path: &str, bytes: &[u8]) -> Result<String, Error> {
    let (parent, file) = path.rsplit_once('/').ok_or(Error::Invalid("target path"))?;
    let physical = format!("targets/{parent}/{}.{file}", &digest(bytes)[7..]);
    RegistryPath::parse(&physical).map_err(|_| Error::Invalid("physical target path limit"))?;
    Ok(physical)
}
