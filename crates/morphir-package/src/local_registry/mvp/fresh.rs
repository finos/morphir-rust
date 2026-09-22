use super::{Error, files, require, store::Backend};
use crate::local_registry::{tuf::*, *};
use package_tough::experimental_storage::MetadataRole;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};

/// Authenticated current metadata, independent of any old lock's evidence pins.
pub(super) struct FreshMetadata {
    root: ProfileMetadata,
    timestamp: ProfileMetadata,
    snapshot: ProfileMetadata,
    targets: ProfileMetadata,
}
impl FreshMetadata {
    pub fn timestamp_digest(&self) -> Digest {
        Digest::parse(&digest(self.timestamp.bytes())).expect("SHA-256 digest")
    }
    pub fn snapshot_digest(&self) -> Digest {
        Digest::parse(&digest(self.snapshot.bytes())).expect("SHA-256 digest")
    }

    pub fn targets(&self) -> &Value {
        &self.targets.document()["signed"]["targets"]
    }
    fn roles(&self) -> [(&str, &ProfileMetadata); 4] {
        [
            ("root", &self.root),
            ("timestamp", &self.timestamp),
            ("snapshot", &self.snapshot),
            ("targets", &self.targets),
        ]
    }
    pub fn evidence(&self) -> Vec<Value> {
        self.roles()
            .into_iter()
            .map(|(role, metadata)| {
                json!({
                    "id": role, "registry": "local", "kind": format!("tuf-{role}"),
                    "path": path(metadata, role), "digest": digest(metadata.bytes()),
                })
            })
            .collect()
    }
    pub fn check_lock_pins(&self, lock: &LibraryLock) -> Result<(), Error> {
        for evidence in lock
            .evidence()
            .iter()
            .filter(|e| e.kind() != EvidenceKind::ReleaseStatement)
        {
            let (role, metadata) = match evidence.kind() {
                EvidenceKind::TufRoot => ("root", &self.root),
                EvidenceKind::TufTimestamp => ("timestamp", &self.timestamp),
                EvidenceKind::TufSnapshot => ("snapshot", &self.snapshot),
                EvidenceKind::TufTargets => ("targets", &self.targets),
                EvidenceKind::ReleaseStatement => unreachable!(),
            };
            require(
                evidence.reference().digest().as_str() == digest(metadata.bytes())
                    && evidence.reference().path().as_str() == path(metadata, role),
                "historical evidence unsupported by MVP; refresh lock metadata pins",
            )?;
        }
        Ok(())
    }
}
fn digest(bytes: &[u8]) -> String {
    crate::digest::Digest::of_bytes(bytes).to_string()
}
fn path(metadata: &ProfileMetadata, role: &str) -> String {
    // decode_profile already established an exact positive integer token. Keep
    // its decimal spelling instead of narrowing unbounded versions to u64.
    format!(
        "metadata/{}.{role}.json",
        metadata.document()["signed"]["version"]
    )
}
fn expires(metadata: &ProfileMetadata, now: jiff::Timestamp) -> Result<(), Error> {
    let time: jiff::Timestamp = metadata.document()["signed"]["expires"]
        .as_str()
        .ok_or(Error::Refused("metadata expiry"))?
        .parse()
        .map_err(|_| Error::Refused("metadata expiry"))?;
    require(time > now, "expired repository metadata")
}
/// Reacquire the complete current view even when Tough stopped at timestamp
/// equality. Retained bytes alone never authorize an operation.
pub(super) async fn authenticate(
    backend: &Arc<Backend>,
    registry: &Path,
    policy: &PolicyRepository,
) -> Result<FreshMetadata, Error> {
    let guard = Arc::new(ProfileAdmission::new(
        policy.clone(),
        backend.binding.clone(),
        backend.clone(),
    )?);
    let base = url::Url::from_directory_path(registry.join("metadata"))
        .map_err(|_| Error::Refused("metadata path"))?;
    guard
        .load_metadata(
            Box::new(files::LocalTransport {
                root: registry.to_owned(),
            }),
            base,
        )
        .await?;
    let snapshot = backend.read().await?;
    let root = decode_profile(&snapshot.state.current_root, TufRole::Root)?;
    expires(&root, backend.binding.fixed_time)?;
    let mut chain = Vec::new();
    for (role, name, spelling) in [
        (TufRole::Timestamp, MetadataRole::Timestamp, "timestamp"),
        (TufRole::Snapshot, MetadataRole::Snapshot, "snapshot"),
        (TufRole::Targets, MetadataRole::Targets, "targets"),
    ] {
        let retained = snapshot
            .state
            .metadata
            .get(&name)
            .ok_or(Error::Refused("incomplete retained metadata"))?;
        let decoded = decode_profile(&retained.bytes, role)?;
        let filename = if role == TufRole::Timestamp {
            "metadata/timestamp.json".into()
        } else {
            path(&decoded, spelling)
        };
        let bytes = files::read(
            registry,
            &filename,
            if role == TufRole::Targets {
                16_777_216
            } else {
                1_048_576
            },
        )?;
        require(
            bytes == retained.bytes,
            "fresh metadata differs from retained view",
        )?;
        backend
            .record(snapshot.state.revision, CandidateEvidence { role, bytes })
            .await?;
        verify_quorum(&root, &decoded)?;
        expires(&decoded, backend.binding.fixed_time)?;
        chain.push(decoded);
    }
    verify_link(&chain[0], &chain[1])?;
    verify_link(&chain[1], &chain[2])?;
    let targets = chain.pop().unwrap();
    let snapshot = chain.pop().unwrap();
    let timestamp = chain.pop().unwrap();
    Ok(FreshMetadata {
        root,
        timestamp,
        snapshot,
        targets,
    })
}
