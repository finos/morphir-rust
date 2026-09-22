use super::{AdmissionError, ProfileAdmission, ProfileTransport, profile_limits};
use package_tough::{
    RepositoryLoader, Transport,
    experimental_storage::{PackageLoadOutcome, Storage},
};
use std::sync::Arc;
use url::Url;

/// Failure of profile admission or the upstream TUF metadata update.
#[derive(Debug, thiserror::Error)]
pub enum MetadataLoadError {
    /// Host profile/context rejected metadata or protected state.
    #[error(transparent)]
    Admission(#[from] AdmissionError),
    /// Upstream crypto, expiry, rollback, transport or protected-store failure.
    #[error(transparent)]
    Update(Box<package_tough::error::Error>),
    /// The initial protected state could not be admitted.
    #[error(transparent)]
    State(#[from] package_tough::experimental_storage::Error),
}
impl ProfileAdmission {
    /// Run the unchanged TUF workflow with strict ingress, fixed time, exact
    /// evidence and the same mandatory guard for storage and admission.
    ///
    /// `Updated` proves metadata admission only. Publisher checks, package bytes,
    /// durable revocations, recovery and grant issuance remain host obligations.
    /// `NoUpdate` does not establish a fresh complete view or permit a new grant.
    /// The returned repository's transport accepts metadata only; package target
    /// acquisition belongs to the separate bounded package-store pipeline.
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use morphir_package::local_registry::{PolicyRepository, tuf::*};
    /// async fn refresh(
    ///     policy: PolicyRepository, marker: OperationBinding,
    ///     backend: Arc<dyn AdmissionBackend>, metadata: url::Url,
    /// ) -> Result<(), MetadataLoadError> {
    ///     let admission = Arc::new(ProfileAdmission::new(policy, marker, backend)?);
    ///     let outcome = admission.load_metadata(
    ///         Box::new(package_tough::FilesystemTransport), metadata,
    ///     ).await?;
    ///     // The caller must handle Updated and NoUpdate before package authorization.
    ///     Ok(())
    /// }
    /// ```
    pub async fn load_metadata(
        self: Arc<Self>,
        source: Box<dyn Transport>,
        metadata_base: Url,
    ) -> Result<PackageLoadOutcome, MetadataLoadError> {
        let root = self.snapshot().await?.provisioned_root;
        let transport = ProfileTransport::new(source, self.clone(), metadata_base.clone())?;
        RepositoryLoader::new(&root, metadata_base.clone(), metadata_base)
            .transport(transport)
            .limits(profile_limits())
            .fixed_time(self.fixed_time())
            .experimental_storage(self.clone(), self)
            .load_package()
            .await
            .map_err(|error| MetadataLoadError::Update(Box::new(error)))
    }
}
