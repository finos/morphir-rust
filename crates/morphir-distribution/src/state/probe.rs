//! Stage under the home filesystem without changing the active store or locks.
use super::*;

impl ExtensionInstaller<'_> {
    /// Verify and stage the selected bytes, then run the host's statement probe.
    ///
    /// The callback may execute only the staged artifact and must return a statement
    /// that agrees with its declaration, or an error. No active store, catalog, or
    /// extension lock is changed until it succeeds. The staged bytes are rehashed
    /// during publication; the original source is never reopened after the probe.
    ///
    /// A caller explicitly skipping execution can retain the declaration:
    /// ```no_run
    /// # async fn example(home: &morphir_common::home::MorphirHome,
    /// # selected: morphir_distribution::ResolvedArtifact, host: &semver::Version)
    /// # -> morphir_distribution::Result<()> {
    /// morphir_distribution::ExtensionInstaller::new(home)
    ///     .install_with_probe(selected, host, async |artifact| {
    ///         Ok(artifact.selected().artifact().declared_statement_record())
    ///     }).await?;
    /// # Ok(()) }
    /// ```
    pub async fn install_with_probe(
        &self,
        selected: ResolvedArtifact,
        host: &Version,
        probe: impl AsyncFnOnce(&VerifiedArtifact) -> Result<StatementRecord>,
    ) -> Result<InstalledExtension> {
        selected.check_host(host)?;
        let staging_root = self.home.temp_dir();
        fs::create_dir_all(&staging_root).map_err(|source| DistributionError::Io {
            path: staging_root.clone(),
            source,
        })?;
        let mut builder = tempfile::Builder::new();
        builder.prefix("morphir-extension-probe-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let staging =
            builder
                .tempdir_in(&staging_root)
                .map_err(|source| DistributionError::Io {
                    path: staging_root,
                    source,
                })?;
        let staging_home = MorphirHome::resolve_from(Some(staging.path().as_os_str()), None)
            .map_err(|error| DistributionError::Probe(error.to_string()))?;
        let mut verified = ArtifactStore::from_home(&staging_home).materialize(selected)?;
        let statement = probe(&verified).await?;
        statement
            .statement()
            .ok_or_else(|| {
                DistributionError::Probe("probe returned no capability statement".into())
            })?
            .check_host(host)?;
        verified.selected.artifact.record_statement(statement);
        // Validate both projections before publishing any bytes to the active home.
        ExtensionLock::from_verified(&verified)?;
        InstalledExtension::from_verified(&verified)?;
        let artifact = &verified.selected.artifact;
        let stored = ArtifactStore::from_home(self.home).materialize_file(
            staging.path(),
            &verified.store_path,
            artifact.digest(),
            artifact.filename(),
            artifact.executable(),
        )?;
        verified.path = stored.path().to_owned();
        verified.store_path = RelativeArtifactPath::from_native_path(stored.store_path())?;
        self.commit_verified(verified, &FilesystemStateWriter, InstallMode::Create)
    }
}
