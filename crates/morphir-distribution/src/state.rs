//! Exact locks, installed catalog state, and offline activation.

use crate::state_io::{
    FilesystemStateWriter, StateGuard, StateWriter, atomic_write_json, commit_state_pair,
    decode_state, encode_json, read_json, read_state_bytes, recover_state_pairs, remove_state_pair,
};
use crate::store::{verify_executable_mode, verify_file};
use crate::{
    ArtifactRuntime, ArtifactSource, ArtifactStore, Capability, DistributionError, ExtensionId,
    IndexProvenance, Platform, RelativeArtifactPath, ResolvedArtifact, Result, Selection,
    Sha256Digest, VerifiedArtifact,
};
use morphir_common::home::MorphirHome;
use morphir_extension_sdk::protocol::SUPPORTED_MEP_VERSIONS;
use morphir_extension_sdk::{ExtensionInfo, ExtensionType};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const EXTENSION_LOCK_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateSchemaEnvelope {
    schema_version: u32,
}

/// Reproducible selection and integrity record for one installed extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionLock {
    schema_version: u32,
    selection: Selection,
    extension_id: ExtensionId,
    name: String,
    version: Version,
    index: IndexProvenance,
    source: ArtifactSource,
    runtime: ArtifactRuntime,
    platform: Platform,
    args: Vec<String>,
    digest: Sha256Digest,
    capabilities: Vec<Capability>,
    mep_versions: Vec<String>,
    executable: bool,
}

impl ExtensionLock {
    fn from_verified(artifact: &VerifiedArtifact) -> Self {
        Self {
            schema_version: EXTENSION_LOCK_SCHEMA_VERSION,
            selection: artifact.selected.selection.clone(),
            extension_id: artifact.selected.release.extension_id().clone(),
            name: artifact.selected.release.name().to_owned(),
            version: artifact.selected.release.version().clone(),
            index: artifact.selected.index.clone(),
            source: artifact.selected.artifact.source().clone(),
            runtime: artifact.selected.artifact.runtime(),
            platform: artifact.selected.artifact.platform().clone(),
            args: artifact.selected.artifact.args().to_vec(),
            digest: artifact.selected.artifact.digest().clone(),
            capabilities: artifact.selected.release.capabilities().to_vec(),
            mep_versions: artifact.selected.release.mep_versions().to_vec(),
            executable: artifact.selected.artifact.executable(),
        }
    }

    /// Return the lock schema version.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Return the requested channel or exact version.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Return the selected extension identity.
    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    /// Return the selected extension display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact selected semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return the exact index identity and history revision.
    pub fn index(&self) -> &IndexProvenance {
        &self.index
    }

    /// Return the controlled artifact source.
    pub fn source(&self) -> &ArtifactSource {
        &self.source
    }

    /// Return the selected artifact runtime.
    pub fn runtime(&self) -> ArtifactRuntime {
        self.runtime
    }

    /// Return the selected artifact platform.
    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// Return immutable arguments passed to the extension process.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Return the verified artifact digest.
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    /// Return the extension capabilities fixed by this lock.
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Return MEP versions fixed by this lock.
    pub fn mep_versions(&self) -> &[String] {
        &self.mep_versions
    }

    /// Return the executable state declared by the exact selection.
    pub fn executable(&self) -> bool {
        self.executable
    }
}

/// Write an exact extension lock after artifact verification.
pub fn write_extension_lock(home: &MorphirHome, artifact: &VerifiedArtifact) -> Result<()> {
    let _transaction = extension_state_guard(home)?;
    let lock = ExtensionLock::from_verified(artifact);
    atomic_write_json(&extension_lock_path(home, &lock.extension_id), &lock)
}

/// Read and validate one exact extension lock.
pub fn read_extension_lock(home: &MorphirHome, id: &ExtensionId) -> Result<ExtensionLock> {
    let _transaction = extension_state_guard(home)?;
    read_extension_lock_unlocked(home, id)
}

fn read_extension_lock_unlocked(home: &MorphirHome, id: &ExtensionId) -> Result<ExtensionLock> {
    let path = extension_lock_path(home, id);
    let bytes = read_state_bytes(&path)?;
    let envelope: StateSchemaEnvelope = decode_state(&path, &bytes)?;
    if envelope.schema_version != EXTENSION_LOCK_SCHEMA_VERSION {
        return Err(DistributionError::UnsupportedStateSchema {
            kind: "extension lock",
            version: envelope.schema_version,
        });
    }
    let lock: ExtensionLock = decode_state(&path, &bytes)?;
    if &lock.extension_id != id {
        return Err(DistributionError::StateMismatch { id: id.clone() });
    }
    Ok(lock)
}

fn extension_lock_path(home: &MorphirHome, id: &ExtensionId) -> PathBuf {
    home.extensions_locks_dir().join(format!("{id}.json"))
}

/// One active extension entry in the durable installed catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstalledExtension {
    extension_id: ExtensionId,
    name: String,
    version: Version,
    runtime: ArtifactRuntime,
    platform: Platform,
    args: Vec<String>,
    digest: Sha256Digest,
    store_path: RelativeArtifactPath,
    capabilities: Vec<Capability>,
    mep_versions: Vec<String>,
    index: IndexProvenance,
    executable: bool,
}

impl InstalledExtension {
    fn from_verified(artifact: &VerifiedArtifact) -> Self {
        Self {
            extension_id: artifact.selected.release.extension_id().clone(),
            name: artifact.selected.release.name().to_owned(),
            version: artifact.selected.release.version().clone(),
            runtime: artifact.selected.artifact.runtime(),
            platform: artifact.selected.artifact.platform().clone(),
            args: artifact.selected.artifact.args().to_vec(),
            digest: artifact.selected.artifact.digest().clone(),
            store_path: artifact.store_path.clone(),
            capabilities: artifact.selected.release.capabilities().to_vec(),
            mep_versions: artifact.selected.release.mep_versions().to_vec(),
            index: artifact.selected.index.clone(),
            executable: artifact.selected.artifact.executable(),
        }
    }

    /// Return the stable extension identity.
    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    /// Return the human-readable extension name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the exact installed version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return the process runtime.
    pub fn runtime(&self) -> ArtifactRuntime {
        self.runtime
    }

    /// Return the installed platform.
    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// Return immutable launch arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Return the verified digest stored in the catalog.
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    /// Return the artifact path relative to Morphir home.
    pub fn store_path(&self) -> &Path {
        self.store_path.as_path()
    }

    /// Return declared extension capabilities.
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Return supported Morphir Extension Protocol versions.
    pub fn mep_versions(&self) -> &[String] {
        &self.mep_versions
    }

    /// Return exact index provenance.
    pub fn index(&self) -> &IndexProvenance {
        &self.index
    }

    /// Return the executable state registered for the artifact.
    pub fn executable(&self) -> bool {
        self.executable
    }

    /// Convert installed discovery metadata to the shared MEP representation.
    pub fn extension_info(&self) -> ExtensionInfo {
        ExtensionInfo {
            id: self.extension_id.to_string(),
            name: self.name.clone(),
            version: self.version.to_string(),
            types: self
                .capabilities
                .iter()
                .copied()
                .map(extension_type)
                .collect(),
            ..ExtensionInfo::default()
        }
    }
}

/// One atomically read and fully validated installed extension state pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExtensionSnapshot {
    installed: InstalledExtension,
    selection: Selection,
}

impl InstalledExtensionSnapshot {
    /// Return the exact installed catalog entry.
    pub fn installed(&self) -> &InstalledExtension {
        &self.installed
    }

    /// Return the channel or exact-version request stored in the exact lock.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogFile {
    schema_version: u32,
    extensions: Vec<InstalledExtension>,
}

/// Durable installed extension catalog.
#[derive(Debug)]
pub struct InstalledCatalog {
    home: MorphirHome,
    extensions: BTreeMap<ExtensionId, InstalledExtension>,
}

impl InstalledCatalog {
    /// Load the durable catalog, or create an empty in-memory catalog if absent.
    pub fn load(home: &MorphirHome) -> Result<Self> {
        let _transaction = extension_state_guard(home)?;
        Self::load_unlocked(home)
    }

    fn load_unlocked(home: &MorphirHome) -> Result<Self> {
        let path = home.extensions_catalog_file();
        if !path.exists() {
            return Ok(Self {
                home: home.clone(),
                extensions: BTreeMap::new(),
            });
        }
        let stored: CatalogFile = read_json(&path)?;
        if stored.schema_version != 1 {
            return Err(DistributionError::UnsupportedStateSchema {
                kind: "installed extension catalog",
                version: stored.schema_version,
            });
        }
        let mut extensions = BTreeMap::new();
        for extension in stored.extensions {
            let id = extension.extension_id.clone();
            if extensions.insert(id.clone(), extension).is_some() {
                return Err(DistributionError::StateMismatch { id });
            }
        }
        Ok(Self {
            home: home.clone(),
            extensions,
        })
    }

    /// Return an installed entry by stable identity.
    pub fn get(&self, id: &ExtensionId) -> Option<&InstalledExtension> {
        self.extensions.get(id)
    }

    /// Return installed entries in stable identity order.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &InstalledExtension> {
        self.extensions.values()
    }

    /// Register verified bytes as the active exact extension.
    ///
    /// The parameter cannot be constructed from an unchecked path. This write
    /// is deliberately the final step of [`ExtensionInstaller::install`].
    pub fn register(&mut self, artifact: VerifiedArtifact) -> Result<InstalledExtension> {
        let _transaction = extension_state_guard(&self.home)?;
        let latest = Self::load_unlocked(&self.home)?;
        let entry = InstalledExtension::from_verified(&artifact);
        let mut next = latest.extensions;
        next.insert(entry.extension_id.clone(), entry.clone());
        let stored = CatalogFile {
            schema_version: 1,
            extensions: next.values().cloned().collect(),
        };
        atomic_write_json(&self.home.extensions_catalog_file(), &stored)?;
        self.extensions = next;
        Ok(entry)
    }
}

/// Ordered verified installation into store, lock, then active catalog.
#[derive(Debug)]
pub struct ExtensionInstaller<'home> {
    home: &'home MorphirHome,
}

impl<'home> ExtensionInstaller<'home> {
    /// Construct an installer for one Morphir home.
    pub fn new(home: &'home MorphirHome) -> Self {
        Self { home }
    }

    /// Materialize verified bytes, write the exact lock, then register them.
    pub fn install(&self, selected: ResolvedArtifact) -> Result<InstalledExtension> {
        self.install_with_writer(selected, &FilesystemStateWriter)
    }

    fn install_with_writer(
        &self,
        selected: ResolvedArtifact,
        writer: &impl StateWriter,
    ) -> Result<InstalledExtension> {
        let verified = ArtifactStore::from_home(self.home).materialize(selected)?;
        let _transaction = extension_state_guard(self.home)?;
        let catalog = InstalledCatalog::load_unlocked(self.home)?;
        let lock = ExtensionLock::from_verified(&verified);
        let entry = InstalledExtension::from_verified(&verified);
        let mut extensions = catalog.extensions;
        extensions.insert(entry.extension_id.clone(), entry.clone());
        let stored = CatalogFile {
            schema_version: 1,
            extensions: extensions.into_values().collect(),
        };
        let lock_bytes = encode_json(&lock)?;
        let catalog_bytes = encode_json(&stored)?;
        commit_state_pair(
            &extension_lock_path(self.home, &entry.extension_id),
            &lock_bytes,
            &self.home.extensions_catalog_file(),
            &catalog_bytes,
            writer,
        )?;
        Ok(entry)
    }
}

/// Remove an installed extension from active state without deleting CAS bytes.
///
/// The returned entry is the exact catalog record that was removed. A missing
/// catalog entry returns [`DistributionError::NotInstalled`].
pub fn uninstall_extension(home: &MorphirHome, id: &ExtensionId) -> Result<InstalledExtension> {
    uninstall_with_writer(home, id, &FilesystemStateWriter)
}

fn uninstall_with_writer(
    home: &MorphirHome,
    id: &ExtensionId,
    writer: &impl StateWriter,
) -> Result<InstalledExtension> {
    let _transaction = extension_state_guard(home)?;
    let catalog = InstalledCatalog::load_unlocked(home)?;
    let mut extensions = catalog.extensions;
    let removed = extensions
        .remove(id)
        .ok_or_else(|| DistributionError::NotInstalled { id: id.clone() })?;
    let stored = CatalogFile {
        schema_version: 1,
        extensions: extensions.into_values().collect(),
    };
    let lock_path = extension_lock_path(home, id);
    let catalog_path = home.extensions_catalog_file();
    remove_state_pair(&lock_path, &catalog_path, &encode_json(&stored)?, writer)?;
    Ok(removed)
}

/// Atomically list installed extensions with their exact requested selections.
///
/// The catalog and every corresponding lock are loaded under one Morphir-home
/// state lock. Every pair must agree before any snapshot is returned.
pub fn list_installed(home: &MorphirHome) -> Result<Vec<InstalledExtensionSnapshot>> {
    list_installed_with_catalog_observer(home, || {})
}

fn list_installed_with_catalog_observer(
    home: &MorphirHome,
    after_catalog: impl FnOnce(),
) -> Result<Vec<InstalledExtensionSnapshot>> {
    let _transaction = extension_state_guard(home)?;
    let catalog = InstalledCatalog::load_unlocked(home)?;
    after_catalog();
    list_installed_catalog_unlocked(home, catalog)
}

fn list_installed_catalog_unlocked(
    home: &MorphirHome,
    catalog: InstalledCatalog,
) -> Result<Vec<InstalledExtensionSnapshot>> {
    catalog
        .extensions
        .into_values()
        .map(|installed| {
            let lock = read_extension_lock_unlocked(home, installed.extension_id())?;
            validate_installed_pair(&installed, &lock)?;
            Ok(InstalledExtensionSnapshot {
                installed,
                selection: lock.selection,
            })
        })
        .collect()
}

#[cfg(test)]
fn list_installed_with_reacquisition_after_catalog(
    home: &MorphirHome,
    after_catalog: impl FnOnce(),
) -> Result<Vec<InstalledExtensionSnapshot>> {
    let catalog = InstalledCatalog::load(home)?;
    after_catalog();
    catalog
        .extensions
        .into_values()
        .map(|installed| {
            let lock = read_extension_lock(home, installed.extension_id())?;
            validate_installed_pair(&installed, &lock)?;
            Ok(InstalledExtensionSnapshot {
                installed,
                selection: lock.selection,
            })
        })
        .collect()
}

/// Offline process activation whose installed bytes have just been rehashed.
#[derive(Debug, Clone)]
pub struct VerifiedProcessArtifact {
    program: PathBuf,
    args: Vec<String>,
    extension_info: ExtensionInfo,
}

impl VerifiedProcessArtifact {
    /// Return the verified absolute process path.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Return immutable installed launch arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Return exact metadata that MEP initialization must reproduce.
    pub fn extension_info(&self) -> &ExtensionInfo {
        &self.extension_info
    }
}

/// Activate one catalog entry without consulting its source index.
///
/// The catalog and exact lock must agree. The artifact is then canonicalized
/// beneath Morphir home and rehashed before this function returns.
pub fn activate_installed(home: &MorphirHome, id: &ExtensionId) -> Result<VerifiedProcessArtifact> {
    let (installed, lock) = {
        let _transaction = extension_state_guard(home)?;
        let catalog = InstalledCatalog::load_unlocked(home)?;
        let installed = catalog
            .get(id)
            .cloned()
            .ok_or_else(|| DistributionError::NotInstalled { id: id.clone() })?;
        let lock = read_extension_lock_unlocked(home, id)?;
        (installed, lock)
    };
    validate_installed_pair(&installed, &lock)?;

    let home_root = fs::canonicalize(home.root()).map_err(|source| DistributionError::Io {
        path: home.root().to_path_buf(),
        source,
    })?;
    let requested = home.root().join(installed.store_path());
    let program = fs::canonicalize(&requested).map_err(|source| DistributionError::Io {
        path: requested,
        source,
    })?;
    if !program.starts_with(&home_root) {
        return Err(DistributionError::InstalledPathEscape {
            path: program,
            root: home_root,
        });
    }
    verify_file(&program, &lock.digest)?;
    verify_executable_mode(&program, lock.executable)?;

    Ok(VerifiedProcessArtifact {
        program,
        args: installed.args.clone(),
        extension_info: installed.extension_info(),
    })
}

fn validate_installed_pair(installed: &InstalledExtension, lock: &ExtensionLock) -> Result<()> {
    if lock.extension_id != installed.extension_id
        || lock.name != installed.name
        || lock.version != installed.version
        || lock.runtime != installed.runtime
        || lock.platform != installed.platform
        || lock.args != installed.args
        || lock.digest != installed.digest
        || lock.capabilities != installed.capabilities
        || lock.mep_versions != installed.mep_versions
        || lock.index != installed.index
        || lock.executable != installed.executable
    {
        return Err(DistributionError::StateMismatch {
            id: installed.extension_id.clone(),
        });
    }
    if !lock
        .mep_versions
        .iter()
        .any(|version| SUPPORTED_MEP_VERSIONS.contains(&version.as_str()))
    {
        return Err(DistributionError::NoCompatibleMepVersion {
            selection: lock.selection.to_string(),
            supported: SUPPORTED_MEP_VERSIONS.join(", "),
        });
    }
    Ok(())
}

fn extension_type(capability: Capability) -> ExtensionType {
    match capability {
        Capability::Frontend => ExtensionType::Frontend,
        Capability::Backend => ExtensionType::Backend,
        Capability::Transform => ExtensionType::Transform,
        Capability::Validator => ExtensionType::Validator,
    }
}

fn extension_state_guard(home: &MorphirHome) -> Result<StateGuard> {
    let guard = StateGuard::acquire(&home.extensions_state_lock_file())?;
    recover_state_pairs(
        &home.extensions_locks_dir(),
        &home.extensions_catalog_file(),
    )?;
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_io::{atomic_write_bytes, remove_file};
    use crate::{Channel, LocalIndex};
    use std::sync::{Mutex, mpsc};
    use std::thread;
    use std::time::Duration;

    struct FailingWriter {
        fail_path: PathBuf,
        fail_after_write: bool,
    }

    impl StateWriter for FailingWriter {
        fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
            if path == self.fail_path {
                if self.fail_after_write {
                    atomic_write_bytes(path, bytes)?;
                }
                return Err(DistributionError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("injected catalog write failure"),
                });
            }
            atomic_write_bytes(path, bytes)
        }
    }

    enum UninstallFailure {
        LockRemoval,
        CatalogWrite,
    }

    struct FailingUninstallWriter {
        failure: UninstallFailure,
        lock_path: PathBuf,
        catalog_path: PathBuf,
    }

    struct PausingCatalogWriter {
        catalog_path: PathBuf,
        reached_catalog: Mutex<Option<mpsc::Sender<()>>>,
        resume: Mutex<mpsc::Receiver<()>>,
    }

    impl StateWriter for PausingCatalogWriter {
        fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
            if path == self.catalog_path {
                self.reached_catalog
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(())
                    .unwrap();
                self.resume.lock().unwrap().recv().unwrap();
            }
            atomic_write_bytes(path, bytes)
        }
    }

    impl StateWriter for FailingUninstallWriter {
        fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
            atomic_write_bytes(path, bytes)?;
            if matches!(self.failure, UninstallFailure::CatalogWrite) && path == self.catalog_path {
                return Err(injected_uninstall_error(path));
            }
            Ok(())
        }

        fn remove(&self, path: &Path) -> Result<()> {
            remove_file(path)?;
            if matches!(self.failure, UninstallFailure::LockRemoval) && path == self.lock_path {
                return Err(injected_uninstall_error(path));
            }
            Ok(())
        }
    }

    #[test]
    fn failed_catalog_commit_restores_previous_state_pair() {
        let root = tempfile::tempdir().unwrap();
        let lock_path = root.path().join("locks/example.json");
        let catalog_path = root.path().join("catalog/extensions.json");
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        fs::create_dir_all(catalog_path.parent().unwrap()).unwrap();
        fs::write(&lock_path, b"old lock\n").unwrap();
        fs::write(&catalog_path, b"old catalog\n").unwrap();

        let writer = FailingWriter {
            fail_path: catalog_path.clone(),
            fail_after_write: true,
        };
        let error = commit_state_pair(
            &lock_path,
            b"new lock\n",
            &catalog_path,
            b"new catalog\n",
            &writer,
        )
        .unwrap_err();

        assert!(error.to_string().contains("injected catalog write failure"));
        assert_eq!(fs::read(lock_path).unwrap(), b"old lock\n");
        assert_eq!(fs::read(catalog_path).unwrap(), b"old catalog\n");
    }

    #[test]
    fn failed_upgrade_restores_the_previously_active_lock_and_catalog() {
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("index");
        let source = index.join("artifacts/example");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(index.join("extensions")).unwrap();
        fs::write(&source, b"example extension").unwrap();
        let digest = Sha256Digest::of_bytes(b"example extension");
        write_release(&index, "1.0.0", &digest);
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let id = ExtensionId::parse("example").unwrap();
        let platform = Platform::new("linux", "x86_64").unwrap();
        let select = || {
            LocalIndex::open(&index)
                .unwrap()
                .resolve(&id, Selection::Channel(Channel::Stable), &platform)
                .unwrap()
        };
        ExtensionInstaller::new(&home).install(select()).unwrap();
        let lock_path = extension_lock_path(&home, &id);
        let catalog_path = home.extensions_catalog_file();
        let previous_lock = fs::read(&lock_path).unwrap();
        let previous_catalog = fs::read(&catalog_path).unwrap();

        write_release(&index, "2.0.0", &digest);
        let writer = FailingWriter {
            fail_path: catalog_path.clone(),
            fail_after_write: false,
        };
        let error = ExtensionInstaller::new(&home)
            .install_with_writer(select(), &writer)
            .unwrap_err();

        assert!(error.to_string().contains("injected catalog write failure"));
        assert_eq!(fs::read(&lock_path).unwrap(), previous_lock);
        assert_eq!(fs::read(&catalog_path).unwrap(), previous_catalog);
        assert_eq!(
            activate_installed(&home, &id)
                .unwrap()
                .extension_info()
                .version,
            "1.0.0"
        );
    }

    #[test]
    fn failed_uninstall_lock_removal_restores_the_previous_state_pair() {
        assert_failed_uninstall_restores_state(UninstallFailure::LockRemoval);
    }

    #[test]
    fn failed_uninstall_catalog_commit_restores_the_previous_state_pair() {
        assert_failed_uninstall_restores_state(UninstallFailure::CatalogWrite);
    }

    #[test]
    fn listing_waits_for_an_in_progress_state_update_and_returns_one_exact_snapshot() {
        let (root, home, id) = installed_extension();
        let index = root.path().join("index");
        let digest = Sha256Digest::of_bytes(b"example extension");
        write_release(&index, "2.0.0", &digest);
        let selected = LocalIndex::open(&index)
            .unwrap()
            .resolve(
                &id,
                Selection::Channel(Channel::Stable),
                &Platform::new("linux", "x86_64").unwrap(),
            )
            .unwrap();
        let (reached_tx, reached_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let writer = PausingCatalogWriter {
            catalog_path: home.extensions_catalog_file(),
            reached_catalog: Mutex::new(Some(reached_tx)),
            resume: Mutex::new(resume_rx),
        };
        let update_home = home.clone();
        let update = thread::spawn(move || {
            ExtensionInstaller::new(&update_home).install_with_writer(selected, &writer)
        });
        reached_rx.recv().unwrap();

        let (listing_started_tx, listing_started_rx) = mpsc::channel();
        let (listing_result_tx, listing_result_rx) = mpsc::channel();
        let listing_home = home.clone();
        let listing = thread::spawn(move || {
            listing_started_tx.send(()).unwrap();
            listing_result_tx
                .send(list_installed(&listing_home))
                .unwrap();
        });
        listing_started_rx.recv().unwrap();
        assert!(
            matches!(
                listing_result_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "listing observed the half-committed catalog/lock pair"
        );

        resume_tx.send(()).unwrap();
        update.join().unwrap().unwrap();
        let snapshots = listing_result_rx.recv().unwrap().unwrap();
        listing.join().unwrap();

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].installed().version().to_string(), "2.0.0");
        assert_eq!(
            snapshots[0].selection(),
            &Selection::Channel(Channel::Stable)
        );
    }

    #[test]
    fn forced_race_breaks_listing_that_reacquires_between_catalog_and_lock_reads() {
        let (root, home, id) = installed_extension();
        let index = root.path().join("index");
        let digest = Sha256Digest::of_bytes(b"example extension");
        write_release(&index, "2.0.0", &digest);
        let selected = LocalIndex::open(&index)
            .unwrap()
            .resolve(
                &id,
                Selection::Channel(Channel::Stable),
                &Platform::new("linux", "x86_64").unwrap(),
            )
            .unwrap();
        let (catalog_read_tx, catalog_read_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let listing_home = home.clone();
        let listing = thread::spawn(move || {
            list_installed_with_reacquisition_after_catalog(&listing_home, || {
                catalog_read_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
            })
        });
        catalog_read_rx.recv().unwrap();

        ExtensionInstaller::new(&home).install(selected).unwrap();
        resume_tx.send(()).unwrap();

        match listing.join().unwrap().unwrap_err() {
            DistributionError::StateMismatch { id: mismatched } => assert_eq!(mismatched, id),
            other => panic!("expected StateMismatch from forced two-acquisition race, got {other}"),
        }
    }

    #[test]
    fn atomic_listing_holds_one_guard_across_the_forced_catalog_lock_race() {
        let (root, home, id) = installed_extension();
        let index = root.path().join("index");
        let digest = Sha256Digest::of_bytes(b"example extension");
        write_release(&index, "2.0.0", &digest);
        let selected = LocalIndex::open(&index)
            .unwrap()
            .resolve(
                &id,
                Selection::Channel(Channel::Stable),
                &Platform::new("linux", "x86_64").unwrap(),
            )
            .unwrap();
        let (catalog_read_tx, catalog_read_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let listing_home = home.clone();
        let listing = thread::spawn(move || {
            list_installed_with_catalog_observer(&listing_home, || {
                catalog_read_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
            })
        });
        catalog_read_rx.recv().unwrap();

        let (update_started_tx, update_started_rx) = mpsc::channel();
        let (update_done_tx, update_done_rx) = mpsc::channel();
        let update_home = home.clone();
        let update = thread::spawn(move || {
            update_started_tx.send(()).unwrap();
            update_done_tx
                .send(ExtensionInstaller::new(&update_home).install(selected))
                .unwrap();
        });
        update_started_rx.recv().unwrap();
        let early_update = update_done_rx.recv_timeout(Duration::from_millis(100));
        let update_was_blocked = matches!(&early_update, Err(mpsc::RecvTimeoutError::Timeout));
        resume_tx.send(()).unwrap();

        let snapshots = listing.join().unwrap().unwrap();
        let update_result = match early_update {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => update_done_rx.recv().unwrap(),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("update thread disconnected"),
        };
        update_result.unwrap();
        update.join().unwrap();

        assert!(
            update_was_blocked,
            "update committed after the old catalog read but before its lock read"
        );
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].installed().version().to_string(), "1.0.0");
        assert_eq!(
            list_installed(&home).unwrap()[0]
                .installed()
                .version()
                .to_string(),
            "2.0.0"
        );
    }

    fn assert_failed_uninstall_restores_state(failure: UninstallFailure) {
        let (root, home, id) = installed_extension();
        let lock_path = extension_lock_path(&home, &id);
        let catalog_path = home.extensions_catalog_file();
        let previous_lock = fs::read(&lock_path).unwrap();
        let previous_catalog = fs::read(&catalog_path).unwrap();
        let writer = FailingUninstallWriter {
            failure,
            lock_path: lock_path.clone(),
            catalog_path: catalog_path.clone(),
        };

        let error = uninstall_with_writer(&home, &id, &writer).unwrap_err();

        assert!(error.to_string().contains("injected uninstall failure"));
        assert_eq!(fs::read(&lock_path).unwrap(), previous_lock);
        assert_eq!(fs::read(&catalog_path).unwrap(), previous_catalog);
        assert!(activate_installed(&home, &id).is_ok());
        drop(root);
    }

    fn installed_extension() -> (tempfile::TempDir, MorphirHome, ExtensionId) {
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("index");
        let source = index.join("artifacts/example");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(index.join("extensions")).unwrap();
        fs::write(&source, b"example extension").unwrap();
        let digest = Sha256Digest::of_bytes(b"example extension");
        write_release(&index, "1.0.0", &digest);
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let id = ExtensionId::parse("example").unwrap();
        let selected = LocalIndex::open(&index)
            .unwrap()
            .resolve(
                &id,
                Selection::Channel(Channel::Stable),
                &Platform::new("linux", "x86_64").unwrap(),
            )
            .unwrap();
        ExtensionInstaller::new(&home).install(selected).unwrap();
        (root, home, id)
    }

    fn injected_uninstall_error(path: &Path) -> DistributionError {
        DistributionError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other("injected uninstall failure"),
        }
    }

    fn write_release(index: &Path, version: &str, digest: &Sha256Digest) {
        let record = serde_json::json!({
            "schemaVersion": 1,
            "id": "example",
            "name": "Example",
            "version": version,
            "channels": ["stable"],
            "mepVersions": ["0.1"],
            "capabilities": ["frontend"],
            "artifacts": [{
                "runtime": "process",
                "platform": { "os": "linux", "arch": "x86_64" },
                "source": { "kind": "local-file", "path": "artifacts/example" },
                "sha256": digest,
                "filename": "example",
                "args": [],
                "executable": false
            }]
        });
        fs::write(
            index.join("extensions/example.jsonl"),
            format!("{record}\n"),
        )
        .unwrap();
    }
}
