use super::*;
use crate::store::read_verified_file;
use morphir_extension_sdk::ExtensionCapabilities;

/// Offline process activation whose installed bytes have just been rehashed.
#[derive(Debug, Clone)]
pub struct VerifiedProcessArtifact {
    program: PathBuf,
    staging_directory: PathBuf,
    bytes: Arc<[u8]>,
    filename: OsString,
    args: Vec<String>,
    extension_info: ExtensionInfo,
    capabilities: ExtensionCapabilities,
    frontend: Option<FrontendRecord>,
    backend: Option<BackendRecord>,
}

impl VerifiedProcessArtifact {
    /// Return the verified absolute process path.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Return Morphir-managed temporary storage on the executable's filesystem.
    pub fn staging_directory(&self) -> &Path {
        &self.staging_directory
    }

    /// Return the exact process bytes whose digest was verified.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the installed executable basename used for private staging.
    pub fn filename(&self) -> &OsStr {
        &self.filename
    }

    /// Return immutable installed launch arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Return exact metadata that MEP initialization must reproduce.
    pub fn extension_info(&self) -> &ExtensionInfo {
        &self.extension_info
    }

    /// Return stored frontend metadata, when declared.
    pub fn frontend(&self) -> Option<&FrontendRecord> {
        self.frontend.as_ref()
    }

    /// Return stored backend metadata, when declared.
    pub fn backend(&self) -> Option<&BackendRecord> {
        self.backend.as_ref()
    }

    /// Return typed capabilities reconstructed from installed metadata.
    pub fn extension_capabilities(&self) -> ExtensionCapabilities {
        self.capabilities.clone()
    }
}

/// Offline WebAssembly activation whose installed bytes have just been rehashed.
#[derive(Debug, Clone)]
pub struct VerifiedWasmArtifact {
    path: PathBuf,
    bytes: Arc<[u8]>,
    extension_info: ExtensionInfo,
    capabilities: ExtensionCapabilities,
    frontend: Option<FrontendRecord>,
    backend: Option<BackendRecord>,
}

impl VerifiedWasmArtifact {
    /// Return the verified absolute WebAssembly module path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the exact WebAssembly bytes whose digest was verified.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume the artifact and return its verified bytes without copying them.
    pub fn into_bytes(self) -> Arc<[u8]> {
        self.bytes
    }

    /// Return exact metadata that MEP initialization must reproduce.
    pub fn extension_info(&self) -> &ExtensionInfo {
        &self.extension_info
    }

    /// Return stored frontend metadata, when declared.
    pub fn frontend(&self) -> Option<&FrontendRecord> {
        self.frontend.as_ref()
    }

    /// Return stored backend metadata, when declared.
    pub fn backend(&self) -> Option<&BackendRecord> {
        self.backend.as_ref()
    }

    /// Return typed capabilities reconstructed from installed metadata.
    pub fn extension_capabilities(&self) -> ExtensionCapabilities {
        self.capabilities.clone()
    }
}

/// A runtime-tagged installed artifact whose exact bytes were reverified offline.
#[derive(Debug, Clone)]
pub enum VerifiedExtensionArtifact {
    /// A native child-process artifact.
    Process(VerifiedProcessArtifact),
    /// A portable WebAssembly module.
    Wasm(VerifiedWasmArtifact),
}

impl VerifiedExtensionArtifact {
    /// Return exact metadata that MEP initialization must reproduce.
    pub fn extension_info(&self) -> &ExtensionInfo {
        match self {
            Self::Process(process) => process.extension_info(),
            Self::Wasm(wasm) => wasm.extension_info(),
        }
    }

    /// Return typed capabilities reconstructed from installed metadata.
    pub fn extension_capabilities(&self) -> ExtensionCapabilities {
        match self {
            Self::Process(process) => process.extension_capabilities(),
            Self::Wasm(wasm) => wasm.extension_capabilities(),
        }
    }
}

/// Activate one catalog entry without consulting its source index.
///
/// The catalog and exact lock must agree. The artifact is then canonicalized
/// beneath Morphir home and rehashed before this function returns.
pub fn activate_installed(
    home: &MorphirHome,
    id: &ExtensionId,
) -> Result<VerifiedExtensionArtifact> {
    let snapshot = {
        let _transaction = extension_state_guard(home)?;
        let catalog = InstalledCatalog::load_unlocked(home)?;
        let installed = catalog
            .get(id)
            .cloned()
            .ok_or_else(|| DistributionError::NotInstalled { id: id.clone() })?;
        let lock = read_extension_lock_unlocked(home, id)?;
        validate_installed_pair(&installed, &lock)?;
        InstalledExtensionSnapshot {
            installed,
            selection: lock.selection,
        }
    };
    activate_installed_snapshot(home, &snapshot)
}

/// Activate the exact atomically validated snapshot selected by a caller.
///
/// Later catalog replacements cannot change the selected version, digest,
/// capabilities, arguments, or frontend and backend metadata used for this activation.
pub fn activate_installed_snapshot(
    home: &MorphirHome,
    snapshot: &InstalledExtensionSnapshot,
) -> Result<VerifiedExtensionArtifact> {
    let installed = snapshot.installed().clone();
    validate_installed_runtime(&installed)?;

    let home_root = fs::canonicalize(home.root()).map_err(|source| DistributionError::Io {
        path: home.root().to_path_buf(),
        source,
    })?;
    let requested = home.root().join(installed.store_path());
    let artifact_path = fs::canonicalize(&requested).map_err(|source| DistributionError::Io {
        path: requested,
        source,
    })?;
    if !artifact_path.starts_with(&home_root) {
        return Err(DistributionError::InstalledPathEscape {
            path: artifact_path,
            root: home_root,
        });
    }
    let bytes = read_verified_file(&artifact_path, &installed.digest, installed.executable)?;
    let filename = artifact_path
        .file_name()
        .expect("an installed artifact path has a filename")
        .to_os_string();

    let extension_info = installed.extension_info();
    let capabilities = installed.extension_capabilities();
    match installed.runtime {
        ArtifactRuntime::Process => Ok(VerifiedExtensionArtifact::Process(
            VerifiedProcessArtifact {
                program: artifact_path,
                staging_directory: home.temp_dir().join("extensions"),
                bytes: bytes.into(),
                filename,
                args: installed.args,
                extension_info,
                capabilities,
                frontend: installed.frontend,
                backend: installed.backend,
            },
        )),
        ArtifactRuntime::Wasm => Ok(VerifiedExtensionArtifact::Wasm(VerifiedWasmArtifact {
            path: artifact_path,
            bytes: bytes.into(),
            extension_info,
            capabilities,
            frontend: installed.frontend,
            backend: installed.backend,
        })),
    }
}
