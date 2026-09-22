//! Freshly authenticated local-directory resolution and restore. This MVP requires an explicit
//! bootstrap, one registry, an absent destination, and caller-controlled roots.
//! Interrupted or failed operations require manual intervention; never delete
//! established trust state to bypass a refusal. No continued-use grant is issued.
mod files;
mod fresh;
mod resolve;
pub use resolve::{ResolveReport, ResolveRequest, resolve};
mod store;
mod verify;
use super::*;
use serde::Serialize;
use std::path::Path;
/// Distinct prerelease capability; this is not provider qualification.
pub const PROFILE: &str = "local-library-mvp";
/// MVP profile revision.
pub const PROFILE_VERSION: &str = "0.1.0-draft.1";
/// Explicit independently trusted initialization inputs.
pub struct InitializeRequest<'a> {
    /// Bounded draft.3 local trust policy.
    pub policy: &'a [u8],
    /// Exact pinned bootstrap envelope.
    pub root: &'a [u8],
    /// New trust directory; existing directories are refused.
    pub state: &'a Path,
}
/// Exact-lock restore inputs. Registry and output roots must be caller-controlled.
pub struct RestoreRequest<'a> {
    /// Current trusted policy, never obtained from the registry.
    pub policy: &'a [u8],
    /// Full draft.3 lock; never modified.
    pub lock: &'a [u8],
    /// Coordinated or immutable local registry directory.
    pub registry: &'a Path,
    /// Previously provisioned trust directory.
    pub state: &'a Path,
    /// Absent destination with an existing parent directory.
    pub output: &'a Path,
}
/// Explicit provisioning result, not a package authorization.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeReport {
    /// Prerelease capability name.
    pub profile: &'static str,
    /// Prerelease capability revision.
    pub profile_version: &'static str,
    /// Provisioned repository identity.
    pub repository: Digest,
}
/// One verified Library in the complete published graph.
#[derive(Debug, Serialize)]
pub struct RestoredPackage {
    /// Exact pinned release.
    pub release: crate::resolution::ReleaseId,
    /// Portable directory relative to the output root.
    pub directory: String,
}
/// Successful fresh authorization and complete graph publication.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    /// Prerelease capability name.
    pub profile: &'static str,
    /// Prerelease capability revision.
    pub profile_version: &'static str,
    /// Published packages, in canonical graph order.
    pub packages: Vec<RestoredPackage>,
}
/// Fail-closed errors; none authorize use of output or retained state.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Unsupported request or violated invariant.
    #[error("fresh restore refused: {0}")]
    Refused(&'static str),
    /// Local I/O failure. A remaining operation marker requires intervention.
    #[error("fresh restore I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Protected database failure.
    #[error("protected trust state: {0}")]
    State(#[from] rusqlite::Error),
    /// Invalid local-registry document.
    #[error("local registry: {0}")]
    Document(#[from] Diagnostic),
    /// Admission failed.
    #[error(transparent)]
    Admission(#[from] tuf::AdmissionError),
    /// Fresh metadata could not be authenticated.
    #[error("fresh metadata authentication: {0:#}")]
    Metadata(#[from] tuf::MetadataLoadError),
    /// Invalid package contents.
    #[error(transparent)]
    Integrity(#[from] crate::InvalidDocument),
    /// Invalid protected serialization.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Built-in package schema failure.
    #[error(transparent)]
    Schema(#[from] crate::schema::SchemaError),
    /// The unchanged pure resolver rejected the authenticated candidate set.
    #[error("resolution rejected: {0:?}")]
    ResolutionRejected(Box<crate::resolution::ResolutionDiagnostic>),
    /// The bounded pure resolver could not complete its operation.
    #[error(transparent)]
    Resolution(#[from] crate::resolution::ResolutionExecutionError),
}
pub(super) fn require(value: bool, message: &'static str) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(Error::Refused(message))
    }
}
pub(super) fn policy(bytes: &[u8]) -> Result<TrustPolicy, Error> {
    let policy = decode_trust_policy(bytes)?;
    require(
        policy.repositories().len() == 1,
        "MVP requires exactly one policy repository",
    )?;
    require(
        policy.continued_use() == ContinuedUse::FreshMetadata,
        "MVP requires fresh-metadata policy",
    )?;
    Ok(policy)
}
/// Provision a new trust directory from a pinned, self-authenticated bootstrap.
///
/// ```no_run
/// use morphir_package::local_registry::mvp::{initialize, InitializeRequest};
/// let policy = std::fs::read("trust-policy.json")?;
/// let root = std::fs::read("root.json")?;
/// initialize(InitializeRequest { policy: &policy, root: &root, state: "trust".as_ref() })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn initialize(request: InitializeRequest<'_>) -> Result<InitializeReport, Error> {
    let policy = policy(request.policy)?;
    let repository = &policy.repositories()[0];
    tuf::AuthenticatedRoots::from_policy(repository, &[request.root.to_vec()])?;
    store::initialize(request.state, repository, request.root)?;
    Ok(InitializeReport {
        profile: PROFILE,
        profile_version: PROFILE_VERSION,
        repository: repository.identity().clone(),
    })
}
/// Authenticate fresh metadata and publishers, verify the entire locked graph,
/// and publish once into an absent output. Every invocation reauthorizes use.
///
/// ```no_run
/// use morphir_package::local_registry::mvp::{
///     initialize, restore, InitializeRequest, RestoreRequest,
/// };
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let policy = std::fs::read("trust-policy.json")?;
/// let root = std::fs::read("root.json")?;
/// let lock = std::fs::read("morphir.lock")?;
/// initialize(InitializeRequest {
///     policy: &policy, root: &root, state: "trust".as_ref(),
/// })?;
/// std::fs::create_dir_all("consumer")?;
/// let report = restore(RestoreRequest {
///     policy: &policy, lock: &lock, registry: "registry".as_ref(),
///     state: "trust".as_ref(), output: "consumer/libraries".as_ref(),
/// }).await?;
/// assert!(!report.packages.is_empty());
/// # Ok(())
/// # }
/// ```
pub async fn restore(request: RestoreRequest<'_>) -> Result<RestoreReport, Error> {
    restore_at(request, jiff::Timestamp::now()).await
}
async fn restore_at(
    request: RestoreRequest<'_>,
    now: jiff::Timestamp,
) -> Result<RestoreReport, Error> {
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
    let packages = verify::graph(
        &backend,
        &registry,
        &policy,
        &lock,
        fresh.targets(),
        stage.path(),
    )?;
    backend.authorized()?;
    // The provider contract requires callers to coordinate writers to the output root.
    files::absent(request.output)?;
    files::promote(stage.path(), request.output)?;
    if let Err(error) = backend.finish() {
        let _ = std::fs::remove_dir_all(request.output);
        return Err(error);
    }
    Ok(RestoreReport {
        profile: PROFILE,
        profile_version: PROFILE_VERSION,
        packages,
    })
}

#[cfg(test)]
mod tests;
