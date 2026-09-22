use super::*;
/// Metadata-only refresh inputs for one provisioned local registry.
pub struct RefreshRequest<'a> {
    /// Current independently trusted policy.
    pub policy: &'a [u8],
    /// Coordinated or immutable local registry directory.
    pub registry: &'a Path,
    /// Previously provisioned trust directory.
    pub state: &'a Path,
}
/// Authenticated metadata observation, never a package authorization.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    /// Prerelease capability name.
    pub profile: &'static str,
    /// Prerelease capability revision.
    pub profile_version: &'static str,
    /// Validated local registry alias.
    pub registry: LocalId,
    /// Digest of the exact accepted timestamp envelope bytes.
    pub timestamp_digest: Digest,
    /// Digest of the exact accepted snapshot envelope bytes.
    pub snapshot_digest: Digest,
}
/// Authenticate and retain current metadata without acquiring any package.
/// The report observes metadata only: it contains no graph, package grant or
/// package-readiness claim. No lock is read or written. Every invocation checks
/// the complete current chain, including repeated timestamps. Unsupported
/// revocation transitions refuse and retain the operation marker.
///
/// ```no_run
/// use morphir_package::local_registry::mvp::{refresh, RefreshRequest};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let policy = std::fs::read("trust-policy.json")?;
/// let report = refresh(RefreshRequest {
///     policy: &policy, registry: "registry".as_ref(),
///     state: "initialized-trust".as_ref(),
/// }).await?;
/// assert_eq!(report.registry.as_str(), "local");
/// # Ok(())
/// # }
/// ```
pub async fn refresh(request: RefreshRequest<'_>) -> Result<RefreshReport, Error> {
    refresh_at(request, jiff::Timestamp::now()).await
}
pub(super) async fn refresh_at(
    request: RefreshRequest<'_>,
    now: jiff::Timestamp,
) -> Result<RefreshReport, Error> {
    let policy = policy(request.policy)?;
    let registry = files::directory(request.registry)?;
    let backend = store::Backend::begin(
        request.state,
        &policy.repositories()[0],
        now,
        request.policy.len(),
    )?;
    let fresh = fresh::authenticate(&backend, &registry, &policy.repositories()[0]).await?;
    super::declarations::check_refresh(fresh.targets())?;
    let report = RefreshReport {
        profile: PROFILE,
        profile_version: PROFILE_VERSION,
        registry: LocalId::parse("local").expect("fixed validated alias"),
        timestamp_digest: fresh.timestamp_digest(),
        snapshot_digest: fresh.snapshot_digest(),
    };
    backend.accept_operation_time()?;
    backend.finish()?;
    Ok(report)
}

#[cfg(test)]
mod tests;
