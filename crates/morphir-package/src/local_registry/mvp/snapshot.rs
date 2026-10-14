//! In-process capture of the original bytes accepted by one fresh restore.

use super::{
    Error, PROFILE, PROFILE_VERSION, RestoreReport, RestoreRequest, files, fresh, policy, require,
    store, verify,
};
use crate::{
    digest::Digest,
    library::LibraryInput,
    local_registry::{RegistryPath, decode_library_lock},
    resolution::ReleaseId,
};
use std::{fmt, path::Path};

/// Exact inventoried bytes retained from verification, without reopening a path.
/// Neither this type nor its containing snapshot can be constructed by callers.
pub struct SnapshotFile {
    path: RegistryPath,
    digest: Digest,
    bytes: Vec<u8>,
}

impl SnapshotFile {
    /// Validated portable path relative to its Library bundle.
    pub fn path(&self) -> &RegistryPath {
        &self.path
    }
    /// SHA-256 of these exact bytes, not a normalized or reserialized document.
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
    /// The original bytes checked against the authenticated content inventory.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for SnapshotFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotFile")
            .field("path", &self.path)
            .field("digest", &self.digest)
            .field("length", &self.bytes.len())
            .finish()
    }
}

/// One exact locked Library, including its native payload and context inventory.
/// Native metadata stays in its original payload; no projection is manufactured.
pub struct SnapshotLibrary {
    release: ReleaseId,
    manifest: Vec<u8>,
    payload: SnapshotFile,
    contexts: Vec<SnapshotFile>,
}

impl SnapshotLibrary {
    fn capture(release: ReleaseId, input: LibraryInput) -> Result<Self, Error> {
        let (manifest, files) = input.into_parts();
        // The complete graph verifier has already checked the manifest and exact
        // inventory. Read only retained memory to classify its verified members.
        let document: serde_json::Value = serde_json::from_str(&manifest)?;
        let payload_path = document["ir"]["payload"]["path"]
            .as_str()
            .ok_or(Error::Refused("verified payload path missing"))?;
        let mut payload = None;
        let mut contexts = Vec::new();
        for (path, bytes) in files {
            let is_payload = path == payload_path;
            let file = SnapshotFile {
                path: RegistryPath::parse(&path)
                    .map_err(|_| Error::Refused("unsafe verified file path"))?,
                digest: Digest::of_bytes(&bytes),
                bytes,
            };
            if is_payload {
                payload = Some(file);
            } else {
                contexts.push(file);
            }
        }
        Ok(Self {
            release,
            manifest: manifest.into_bytes(),
            payload: payload.ok_or(Error::Refused("verified payload missing"))?,
            contexts,
        })
    }

    /// Exact release authenticated and matched to its locked graph node.
    pub fn release(&self) -> &ReleaseId {
        &self.release
    }
    /// Original manifest bytes, before normalization or reserialization.
    /// Its normalized manifest/content identities remain distinct from this hash.
    pub fn manifest(&self) -> &[u8] {
        &self.manifest
    }
    /// Original IR payload and its exact byte revision.
    pub fn payload(&self) -> &SnapshotFile {
        &self.payload
    }
    /// All context resources declared by the verified manifest, without fetching
    /// additional files or following ambient context locations.
    pub fn context_resources(&self) -> &[SnapshotFile] {
        &self.contexts
    }
}

impl fmt::Debug for SnapshotLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotLibrary")
            .field("release", &self.release)
            .field("manifest_length", &self.manifest.len())
            .field("payload", &self.payload)
            .field("contexts", &self.contexts)
            .finish()
    }
}

/// Immutable retained bytes from one successfully finalized fresh restore.
///
/// This is an in-process result, not a wire protocol, persisted authorization or
/// qualified provider. A later operation must call [`restore_snapshot`] again.
/// The original native bytes require a separately checked semantic projection
/// before use by a consumer with a different IR codec. No deserialize/import
/// constructor is provided for treating stored data as a fresh restore result.
///
/// ```compile_fail
/// use morphir_package::local_registry::mvp::RestoredSnapshot;
/// let forged = serde_json::from_str::<RestoredSnapshot>("{}");
/// ```
#[derive(Debug)]
pub struct RestoredSnapshot {
    report: RestoreReport,
    root: ReleaseId,
    policy_digest: Digest,
    lock_digest: Digest,
    libraries: Vec<SnapshotLibrary>,
}

impl RestoredSnapshot {
    /// Existing restore report, with the same profile and publication semantics.
    pub fn report(&self) -> &RestoreReport {
        &self.report
    }
    /// Exact graph-root release selected by the full lock.
    pub fn root(&self) -> &ReleaseId {
        &self.root
    }
    /// Identity of the exact independently supplied policy bytes.
    pub fn policy_digest(&self) -> &Digest {
        &self.policy_digest
    }
    /// Identity of the exact full-lock bytes replayed by this operation.
    pub fn lock_digest(&self) -> &Digest {
        &self.lock_digest
    }
    /// Complete verified graph in the same order as the restore report.
    pub fn libraries(&self) -> &[SnapshotLibrary] {
        &self.libraries
    }
    pub(super) fn into_report(self) -> RestoreReport {
        self.report
    }
}

/// Restore and retain the same bytes verified by the complete fresh operation.
///
/// Uses the existing MVP's caller-controlled registry/output roots, explicit
/// initialized state, budgets, refusal and publication rules. Only after graph
/// verification, publication and successful backend finalization can the caller
/// obtain a snapshot. No files are reopened to construct the result.
///
/// ```no_run
/// use morphir_package::local_registry::mvp::{restore_snapshot, RestoreRequest};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let policy = std::fs::read("trust-policy.json")?;
/// let lock = std::fs::read("morphir.lock")?;
/// let snapshot = restore_snapshot(RestoreRequest {
///     policy: &policy, lock: &lock, registry: "registry".as_ref(),
///     state: "initialized-trust".as_ref(), output: "consumer/libraries".as_ref(),
/// }).await?;
/// for library in snapshot.libraries() {
///     let original_ir = library.payload().bytes();
///     assert!(!original_ir.is_empty());
/// }
/// # Ok(())
/// # }
/// ```
pub async fn restore_snapshot(request: RestoreRequest<'_>) -> Result<RestoredSnapshot, Error> {
    restore_at(request, jiff::Timestamp::now()).await
}

pub(super) async fn restore_at(
    request: RestoreRequest<'_>,
    now: jiff::Timestamp,
) -> Result<RestoredSnapshot, Error> {
    let policy = policy(request.policy)?;
    let lock = decode_library_lock(request.lock)?;
    require(
        lock.registries().len() == 1,
        "MVP requires exactly one lock registry",
    )?;
    files::absent(request.output)?;
    let registry = files::directory(request.registry)?;
    let backend = store::Backend::begin(
        request.state,
        &policy.repositories()[0],
        now,
        request.policy.len() + request.lock.len(),
    )?;
    let fresh = fresh::authenticate(&backend, &registry, &policy.repositories()[0]).await?;
    fresh.check_lock_pins(&lock)?;
    let parent = request
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".morphir-restore-")
        .tempdir_in(parent)?;
    let verified = verify::graph(
        &backend,
        &registry,
        &policy,
        &lock,
        fresh.targets(),
        stage.path(),
        verify::Selection::ExactLocked,
    )?;
    let mut packages = Vec::new();
    let libraries = verified
        .into_iter()
        .map(|library| {
            let captured =
                SnapshotLibrary::capture(library.package.release.clone(), library.input)?;
            packages.push(library.package);
            Ok(captured)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    backend.accept_operation_time()?;
    publish(&backend, stage.path(), request.output)?;
    Ok(RestoredSnapshot {
        report: RestoreReport {
            profile: PROFILE,
            profile_version: PROFILE_VERSION,
            packages,
        },
        root: lock.graph().root().clone(),
        policy_digest: Digest::of_bytes(request.policy),
        lock_digest: Digest::of_bytes(request.lock),
        libraries,
    })
}

fn publish(backend: &store::Backend, stage: &Path, output: &Path) -> Result<(), Error> {
    // The provider contract requires callers to coordinate writers to the output root.
    files::absent(output)?;
    files::promote(stage, output)?;
    if let Err(error) = backend.finish() {
        let _ = std::fs::remove_dir_all(output);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
