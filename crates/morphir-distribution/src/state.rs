//! Exact locks, installed catalog state, and offline activation.

use crate::store::verify_file;
use crate::{
    ArtifactRuntime, ArtifactSource, ArtifactStore, Capability, DistributionError, ExtensionId,
    IndexProvenance, Platform, RelativeArtifactPath, ResolvedArtifact, Result, Selection,
    Sha256Digest, VerifiedArtifact,
};
use morphir_common::home::MorphirHome;
use morphir_extension_sdk::{ExtensionInfo, ExtensionType};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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
    digest: Sha256Digest,
}

impl ExtensionLock {
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

    /// Return the verified artifact digest.
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }
}

/// Write an exact extension lock after artifact verification.
pub fn write_extension_lock(home: &MorphirHome, artifact: &VerifiedArtifact) -> Result<()> {
    let lock = ExtensionLock {
        schema_version: 1,
        selection: artifact.selected.selection.clone(),
        extension_id: artifact.selected.release.extension_id().clone(),
        name: artifact.selected.release.name().to_owned(),
        version: artifact.selected.release.version().clone(),
        index: artifact.selected.index.clone(),
        source: artifact.selected.artifact.source().clone(),
        runtime: artifact.selected.artifact.runtime(),
        platform: artifact.selected.artifact.platform().clone(),
        digest: artifact.selected.artifact.digest().clone(),
    };
    atomic_write_json(&extension_lock_path(home, &lock.extension_id), &lock)
}

/// Read and validate one exact extension lock.
pub fn read_extension_lock(home: &MorphirHome, id: &ExtensionId) -> Result<ExtensionLock> {
    let path = extension_lock_path(home, id);
    let lock: ExtensionLock = read_json(&path)?;
    if lock.schema_version != 1 {
        return Err(DistributionError::UnsupportedStateSchema {
            kind: "extension lock",
            version: lock.schema_version,
        });
    }
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
}

impl InstalledExtension {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogFile {
    schema_version: u32,
    extensions: Vec<InstalledExtension>,
}

/// Durable installed extension catalog.
#[derive(Debug)]
pub struct InstalledCatalog {
    path: PathBuf,
    extensions: BTreeMap<ExtensionId, InstalledExtension>,
}

impl InstalledCatalog {
    /// Load the durable catalog, or create an empty in-memory catalog if absent.
    pub fn load(home: &MorphirHome) -> Result<Self> {
        let path = home.extensions_catalog_file();
        if !path.exists() {
            return Ok(Self {
                path,
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
        Ok(Self { path, extensions })
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
        let entry = InstalledExtension {
            extension_id: artifact.selected.release.extension_id().clone(),
            name: artifact.selected.release.name().to_owned(),
            version: artifact.selected.release.version().clone(),
            runtime: artifact.selected.artifact.runtime(),
            platform: artifact.selected.artifact.platform().clone(),
            args: artifact.selected.artifact.args().to_vec(),
            digest: artifact.selected.artifact.digest().clone(),
            store_path: artifact.store_path,
            capabilities: artifact.selected.release.capabilities().to_vec(),
            mep_versions: artifact.selected.release.mep_versions().to_vec(),
            index: artifact.selected.index.clone(),
        };
        let mut next = self.extensions.clone();
        next.insert(entry.extension_id.clone(), entry.clone());
        let stored = CatalogFile {
            schema_version: 1,
            extensions: next.values().cloned().collect(),
        };
        atomic_write_json(&self.path, &stored)?;
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
        let verified = ArtifactStore::from_home(self.home).materialize(selected)?;
        write_extension_lock(self.home, &verified)?;
        InstalledCatalog::load(self.home)?.register(verified)
    }
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
    let catalog = InstalledCatalog::load(home)?;
    let installed = catalog
        .get(id)
        .cloned()
        .ok_or_else(|| DistributionError::NotInstalled { id: id.clone() })?;
    let lock = read_extension_lock(home, id)?;
    if lock.extension_id != installed.extension_id
        || lock.name != installed.name
        || lock.version != installed.version
        || lock.runtime != installed.runtime
        || lock.platform != installed.platform
        || lock.digest != installed.digest
        || lock.index != installed.index
    {
        return Err(DistributionError::StateMismatch { id: id.clone() });
    }

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

    Ok(VerifiedProcessArtifact {
        program,
        args: installed.args.clone(),
        extension_info: installed.extension_info(),
    })
}

fn extension_type(capability: Capability) -> ExtensionType {
    match capability {
        Capability::Frontend => ExtensionType::Frontend,
        Capability::Backend => ExtensionType::Backend,
        Capability::Transform => ExtensionType::Transform,
        Capability::Validator => ExtensionType::Validator,
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|source| DistributionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| DistributionError::InvalidState {
        path: path.to_path_buf(),
        source,
    })
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().expect("durable state path has a parent");
    fs::create_dir_all(parent).map_err(|source| DistributionError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut staged = tempfile::Builder::new()
        .prefix(".stage-")
        .tempfile_in(parent)
        .map_err(|source| DistributionError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    serde_json::to_writer_pretty(staged.as_file_mut(), value)
        .map_err(DistributionError::StateEncoding)?;
    staged
        .as_file_mut()
        .write_all(b"\n")
        .and_then(|()| staged.as_file_mut().flush())
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|source| DistributionError::Io {
            path: staged.path().to_path_buf(),
            source,
        })?;
    staged
        .persist(path)
        .map_err(|error| DistributionError::Io {
            path: path.to_path_buf(),
            source: error.error,
        })?;
    Ok(())
}
