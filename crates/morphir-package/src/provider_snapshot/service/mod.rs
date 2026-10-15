//! One-shot binary Ion boundary around the shared, freshly authenticated restore.
//! The caller owns executable qualification, project grants, process deadlines,
//! cancellation and scratch disposal. This service never initializes trust state.
mod decode;
mod encode;

use super::project_library;
use crate::local_registry::{decode_library_lock, mvp};

/// Exact one-shot service protocol identity. It is not an executable attestation.
pub const SERVICE_ID: &str = "morphir-package/provider-snapshot/0.1.0-draft.1";
/// Maximum binary request frame, including inline policy and lock bytes.
pub const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
/// Maximum binary response frame, including the complete retained graph.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum exported graph size for this transport profile.
pub const MAX_LIBRARIES: usize = 128;
/// Maximum contexts per Library for this transport profile.
pub const MAX_CONTEXTS: usize = 256;

/// Maximum contexts across the complete graph for bounded consumer decoding.
pub const MAX_TOTAL_CONTEXTS: usize = 1024;

/// Fail-closed service errors. Display intentionally excludes payloads and paths.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// Malformed, oversized, text, multiply framed or wrongly annotated Ion.
    #[error("metadata.restore_invalid_frame")]
    InvalidFrame,
    /// Closed request shape or a validated input was rejected.
    #[error("metadata.restore_invalid_request")]
    InvalidRequest,
    /// The exact requested release is not the full lock's root.
    #[error("metadata.restore_release_mismatch")]
    ReleaseMismatch,
    /// The complete snapshot cannot fit this transport profile.
    #[error("metadata.restore_response_limit")]
    ResponseLimit,
    /// Fresh restoration refused. No successful response is returned.
    #[error("metadata.restore_authentication_failed")]
    Restore(#[source] mvp::Error),
    /// The native root cannot be represented by the selected checked core codec.
    #[error("metadata.restore_projection_unsupported")]
    Projection(#[source] super::ProjectionError),
}

/// Process exactly one bounded binary Ion request and return one binary response.
///
/// Capability requests have no restore effects. Snapshot requests replay the full
/// lock through `restore_snapshot` on every call, then export only its retained
/// buffers. A prior response never authorizes another call. Registry/state/output
/// roots must be explicitly granted and caller-controlled under the shared MVP.
/// Authentication can update trust state even if later projection/export refuses.
///
/// ```no_run
/// # async fn example(request: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
/// let response = morphir_package::provider_snapshot::service::serve_request(request).await?;
/// assert!(response.starts_with(&[0xe0, 1, 0, 0xea]));
/// # Ok(()) }
/// ```
pub async fn serve_request(bytes: &[u8]) -> Result<Vec<u8>, ServiceError> {
    match decode::request(bytes)? {
        decode::Request::Capabilities(correlation) => encode::capabilities(correlation),
        decode::Request::Restore(request) => {
            let lock =
                decode_library_lock(&request.lock).map_err(|_| ServiceError::InvalidRequest)?;
            if lock.graph().root() != &request.release {
                return Err(ServiceError::ReleaseMismatch);
            }
            let snapshot = mvp::restore_snapshot(mvp::RestoreRequest {
                policy: &request.policy,
                lock: &request.lock,
                registry: &request.registry,
                state: &request.state,
                output: &request.output,
            })
            .await
            .map_err(ServiceError::Restore)?;
            let root = snapshot
                .libraries()
                .iter()
                .find(|library| library.release() == snapshot.root())
                .ok_or(ServiceError::ReleaseMismatch)?;
            let projection = project_library(root).map_err(ServiceError::Projection)?;
            encode::snapshot(request.correlation, &snapshot, &projection)
        }
    }
}
