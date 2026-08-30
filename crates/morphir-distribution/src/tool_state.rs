//! Exact tool locks, active catalog state, and offline launch verification.

use crate::state_io::{
    FilesystemStateWriter, StateGuard, StateWriter, commit_state_pair, decode_state, encode_json,
    read_json, read_state_bytes,
};
use crate::store::{add_owner_executable, hash_file, verify_executable_mode, verify_file};
use crate::{
    ArchiveFormat, ArtifactFilename, ArtifactStore, DistributionError, DownloadedToolArtifact,
    Platform, RelativeArtifactPath, ResolvedTrustedToolArtifact, Result, Selection, Sha256Digest,
    ToolId, ToolReleaseStatus,
};
use morphir_common::home::MorphirHome;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const TOOL_LOCK_SCHEMA_VERSION: u32 = 1;
const TOOL_CATALOG_SCHEMA_VERSION: u32 = 1;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateSchemaEnvelope {
    schema_version: u32,
}

/// Verified immutable bytes and authenticated metadata ready for catalog activation.
///
/// Fields are private so durable state cannot be built from an unchecked path.
#[derive(Debug)]
pub struct VerifiedToolPackage {
    selection: Selection,
    tool_id: ToolId,
    tool_name: String,
    version: Version,
    status: ToolReleaseStatus,
    platform: Platform,
    digest: Sha256Digest,
    length: u64,
    targets_version: u64,
    target_path: RelativeArtifactPath,
    store_path: RelativeArtifactPath,
    args: Vec<String>,
    files: Vec<ToolPackageFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolPackageFile {
    path: RelativeArtifactPath,
    digest: Sha256Digest,
    length: u64,
    executable: bool,
}

/// Materialization boundary for authenticated raw executable tool targets.
#[derive(Debug)]
pub struct ToolPackageStore<'home> {
    home: &'home MorphirHome,
}

impl<'home> ToolPackageStore<'home> {
    /// Construct a tool package store for one Morphir Home.
    pub fn new(home: &'home MorphirHome) -> Self {
        Self { home }
    }

    /// Reverify and publish a raw executable, AppImage, or ZIP package into the tool CAS.
    pub fn prepare(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: DownloadedToolArtifact,
    ) -> Result<VerifiedToolPackage> {
        let format = resolved.artifact().archive().format();
        match format {
            ArchiveFormat::Raw | ArchiveFormat::Appimage => {
                self.prepare_raw(resolved, downloaded.into_path())
            }
            ArchiveFormat::Zip => self.prepare_zip(resolved, downloaded.into_path()),
            ArchiveFormat::TarGzip => Err(DistributionError::UnsupportedToolArchive {
                format: "tar-gzip".to_owned(),
            }),
        }
    }

    fn prepare_raw(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: PathBuf,
    ) -> Result<VerifiedToolPackage> {
        let source_root = downloaded
            .parent()
            .expect("downloaded TUF target has a parent");
        let source_name = portable_filename(&downloaded)?;
        let filename = ArtifactFilename::parse(source_name)?;
        let source = RelativeArtifactPath::parse(source_name)?;
        let entry_point = resolved.artifact().launch().path();
        if entry_point.as_str() != source_name {
            return Err(DistributionError::ToolEntryPointMismatch {
                target: source_name.to_owned(),
                entry_point: entry_point.as_str().to_owned(),
            });
        }
        let stored = ArtifactStore::for_tools(self.home).materialize_file(
            source_root,
            &source,
            resolved.digest(),
            &filename,
            true,
        )?;
        let actual_length = fs::metadata(stored.path())
            .map_err(|source| DistributionError::Io {
                path: stored.path().to_path_buf(),
                source,
            })?
            .len();
        if actual_length != resolved.length() {
            return Err(DistributionError::ToolLengthMismatch {
                path: stored.path().to_path_buf(),
                expected: resolved.length(),
                actual: actual_length,
            });
        }
        let store_path = RelativeArtifactPath::from_native_path(stored.store_path())?;
        let files = vec![ToolPackageFile {
            path: store_path.clone(),
            digest: resolved.digest().clone(),
            length: resolved.length(),
            executable: true,
        }];
        Ok(package_from_resolved(resolved, store_path, files))
    }

    fn prepare_zip(
        &self,
        resolved: ResolvedTrustedToolArtifact,
        downloaded: PathBuf,
    ) -> Result<VerifiedToolPackage> {
        let source_root = downloaded
            .parent()
            .expect("downloaded TUF target has a parent");
        let source_name = portable_filename(&downloaded)?;
        let filename = ArtifactFilename::parse(source_name)?;
        let source = RelativeArtifactPath::parse(source_name)?;
        let stored = ArtifactStore::for_tools(self.home).materialize_file(
            source_root,
            &source,
            resolved.digest(),
            &filename,
            false,
        )?;
        let actual_length = fs::metadata(stored.path())
            .map_err(|source| DistributionError::Io {
                path: stored.path().to_path_buf(),
                source,
            })?
            .len();
        if actual_length != resolved.length() {
            return Err(DistributionError::ToolLengthMismatch {
                path: stored.path().to_path_buf(),
                expected: resolved.length(),
                actual: actual_length,
            });
        }

        let digest_directory = stored
            .path()
            .parent()
            .expect("CAS artifact has a digest directory");
        let destination = digest_directory.join("package");
        let staging = tempfile::Builder::new()
            .prefix(".package-")
            .tempdir_in(digest_directory)
            .map_err(|source| DistributionError::Io {
                path: digest_directory.to_path_buf(),
                source,
            })?;
        let staging_root = staging.path().join("root");
        fs::create_dir(&staging_root).map_err(|source| DistributionError::Io {
            path: staging_root.clone(),
            source,
        })?;
        let relative_files = extract_zip(
            stored.path(),
            &staging_root,
            resolved.artifact().launch().path(),
        )?;
        if destination.exists() {
            verify_relative_files(&destination, &relative_files)?;
        } else if let Err(source) = fs::rename(&staging_root, &destination) {
            if destination.exists() {
                verify_relative_files(&destination, &relative_files)?;
            } else {
                return Err(DistributionError::Io {
                    path: destination,
                    source,
                });
            }
        }

        let program = destination.join(resolved.artifact().launch().path().as_path());
        let store_path = home_relative(self.home, &program)?;
        let files = relative_files
            .into_iter()
            .map(|file| {
                Ok(ToolPackageFile {
                    path: home_relative(self.home, &destination.join(file.path.as_path()))?,
                    digest: file.digest,
                    length: file.length,
                    executable: file.executable,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(package_from_resolved(resolved, store_path, files))
    }
}

fn package_from_resolved(
    resolved: ResolvedTrustedToolArtifact,
    store_path: RelativeArtifactPath,
    files: Vec<ToolPackageFile>,
) -> VerifiedToolPackage {
    VerifiedToolPackage {
        selection: resolved.selection().clone(),
        tool_id: resolved.release().tool_id().clone(),
        tool_name: resolved.release().tool_name().to_owned(),
        version: resolved.release().version().clone(),
        status: resolved.release().status(),
        platform: resolved.artifact().platform().clone(),
        digest: resolved.digest().clone(),
        length: resolved.length(),
        targets_version: resolved.targets_version(),
        target_path: resolved.artifact().target_path().clone(),
        store_path,
        args: resolved.artifact().launch().args().to_vec(),
        files,
    }
}

fn portable_filename(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DistributionError::InvalidValue {
            kind: "downloaded tool filename",
            value: path.to_string_lossy().into_owned(),
            reason: "expected one portable UTF-8 filename",
        })
}

fn home_relative(home: &MorphirHome, path: &Path) -> Result<RelativeArtifactPath> {
    let canonical_home = fs::canonicalize(home.root()).map_err(|source| DistributionError::Io {
        path: home.root().to_path_buf(),
        source,
    })?;
    let relative =
        path.strip_prefix(&canonical_home)
            .map_err(|_| DistributionError::InstalledPathEscape {
                path: path.to_path_buf(),
                root: canonical_home,
            })?;
    RelativeArtifactPath::from_native_path(relative)
}

fn extract_zip(
    archive_path: &Path,
    destination: &Path,
    entry_point: &RelativeArtifactPath,
) -> Result<Vec<ToolPackageFile>> {
    let file = fs::File::open(archive_path).map_err(|source| DistributionError::Io {
        path: archive_path.to_path_buf(),
        source,
    })?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|source| unsafe_archive("", format!("invalid ZIP archive: {source}")))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(unsafe_archive(
            "",
            format!("archive exceeds {MAX_ARCHIVE_ENTRIES} entries"),
        ));
    }

    let mut names = BTreeSet::new();
    let mut unpacked = 0_u64;
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|source| unsafe_archive("", format!("invalid ZIP entry: {source}")))?;
        let raw_name = entry.name().to_owned();
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            unsafe_archive(&raw_name, "entry escapes the package root".to_owned())
        })?;
        let relative = RelativeArtifactPath::from_native_path(&enclosed)
            .map_err(|error| unsafe_archive(&raw_name, error.to_string()))?;
        let collision_key = relative.as_str().to_lowercase();
        if !names.insert(collision_key) {
            return Err(unsafe_archive(
                &raw_name,
                "entry collides with another portable path".to_owned(),
            ));
        }
        let mode_kind = entry.unix_mode().unwrap_or(0) & 0o170000;
        if mode_kind != 0 && mode_kind != 0o040000 && mode_kind != 0o100000 {
            return Err(unsafe_archive(
                &raw_name,
                "links, devices, and special files are not allowed".to_owned(),
            ));
        }
        let output = destination.join(relative.as_path());
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|source| DistributionError::Io {
                path: output,
                source,
            })?;
            continue;
        }
        unpacked = unpacked
            .checked_add(entry.size())
            .ok_or_else(|| unsafe_archive(&raw_name, "unpacked size overflow".to_owned()))?;
        if unpacked > MAX_UNPACKED_BYTES {
            return Err(unsafe_archive(
                &raw_name,
                format!("archive expands beyond {MAX_UNPACKED_BYTES} bytes"),
            ));
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|source| DistributionError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut output_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .map_err(|source| DistributionError::Io {
                path: output.clone(),
                source,
            })?;
        io::copy(&mut entry, &mut output_file).map_err(|source| DistributionError::Io {
            path: output.clone(),
            source,
        })?;
        output_file
            .sync_all()
            .map_err(|source| DistributionError::Io {
                path: output.clone(),
                source,
            })?;
        let executable = &relative == entry_point;
        if executable {
            add_owner_executable(&output)?;
        }
        files.push(ToolPackageFile {
            path: relative,
            digest: hash_file(&output)?,
            length: fs::metadata(&output)
                .map_err(|source| DistributionError::Io {
                    path: output.clone(),
                    source,
                })?
                .len(),
            executable,
        });
    }
    if !files.iter().any(|file| &file.path == entry_point) {
        return Err(unsafe_archive(
            entry_point.as_str(),
            "declared launch entry point is missing".to_owned(),
        ));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn verify_relative_files(root: &Path, files: &[ToolPackageFile]) -> Result<()> {
    for file in files {
        let path = root.join(file.path.as_path());
        verify_one_file(&path, &file.digest, file.length, file.executable)?;
    }
    Ok(())
}

fn unsafe_archive(entry: impl Into<String>, reason: impl Into<String>) -> DistributionError {
    DistributionError::UnsafeToolArchive {
        entry: entry.into(),
        reason: reason.into(),
    }
}

/// Reproducible selection and integrity record for one active tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolLock {
    schema_version: u32,
    selection: Selection,
    tool_id: ToolId,
    tool_name: String,
    version: Version,
    status: ToolReleaseStatus,
    platform: Platform,
    digest: Sha256Digest,
    length: u64,
    targets_version: u64,
    target_path: RelativeArtifactPath,
    store_path: RelativeArtifactPath,
    args: Vec<String>,
    files: Vec<ToolPackageFile>,
}

impl ToolLock {
    fn from_package(package: &VerifiedToolPackage) -> Self {
        Self {
            schema_version: TOOL_LOCK_SCHEMA_VERSION,
            selection: package.selection.clone(),
            tool_id: package.tool_id.clone(),
            tool_name: package.tool_name.clone(),
            version: package.version.clone(),
            status: package.status,
            platform: package.platform.clone(),
            digest: package.digest.clone(),
            length: package.length,
            targets_version: package.targets_version,
            target_path: package.target_path.clone(),
            store_path: package.store_path.clone(),
            args: package.args.clone(),
            files: package.files.clone(),
        }
    }

    /// Return the requested channel or exact version.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Return the exact selected tool identity.
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    /// Return the exact selected semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return the authenticated artifact digest.
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }
}

/// One immutable installed tool release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstalledTool {
    tool_id: ToolId,
    tool_name: String,
    version: Version,
    status: ToolReleaseStatus,
    platform: Platform,
    digest: Sha256Digest,
    length: u64,
    targets_version: u64,
    target_path: RelativeArtifactPath,
    store_path: RelativeArtifactPath,
    args: Vec<String>,
    files: Vec<ToolPackageFile>,
}

impl InstalledTool {
    fn from_package(package: &VerifiedToolPackage) -> Self {
        Self {
            tool_id: package.tool_id.clone(),
            tool_name: package.tool_name.clone(),
            version: package.version.clone(),
            status: package.status,
            platform: package.platform.clone(),
            digest: package.digest.clone(),
            length: package.length,
            targets_version: package.targets_version,
            target_path: package.target_path.clone(),
            store_path: package.store_path.clone(),
            args: package.args.clone(),
            files: package.files.clone(),
        }
    }

    /// Return the stable tool identity.
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    /// Return the human-readable tool name.
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Return the exact installed semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Return the installed platform.
    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// Return the artifact digest authenticated at installation time.
    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    /// Return the installed program path relative to Morphir Home.
    pub fn store_path(&self) -> &Path {
        self.store_path.as_path()
    }

    /// Return fixed arguments prepended during launch.
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolCatalogEntry {
    active: InstalledTool,
    rollback: Vec<InstalledTool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ToolCatalogFile {
    schema_version: u32,
    tools: Vec<ToolCatalogEntry>,
}

/// One atomically read active tool, its exact selection, and retained rollback releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledToolSnapshot {
    active: InstalledTool,
    rollback: Vec<InstalledTool>,
    selection: Selection,
}

impl InstalledToolSnapshot {
    /// Return the active exact release.
    pub fn active(&self) -> &InstalledTool {
        &self.active
    }

    /// Return inactive releases retained for explicit rollback and pruning policy.
    pub fn rollback(&self) -> &[InstalledTool] {
        &self.rollback
    }

    /// Return the channel or exact-version request stored in the exact lock.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
}

/// Transactional activation of one already verified tool package.
#[derive(Debug)]
pub struct ToolInstaller<'home> {
    home: &'home MorphirHome,
}

impl<'home> ToolInstaller<'home> {
    /// Construct an installer for one Morphir Home.
    pub fn new(home: &'home MorphirHome) -> Self {
        Self { home }
    }

    /// Verify the candidate again, then atomically replace its lock and catalog entry.
    pub fn install(&self, package: VerifiedToolPackage) -> Result<InstalledTool> {
        self.install_with_writer(package, &FilesystemStateWriter)
    }

    fn install_with_writer(
        &self,
        package: VerifiedToolPackage,
        writer: &impl StateWriter,
    ) -> Result<InstalledTool> {
        verify_package(self.home, &package)?;
        if package.status == ToolReleaseStatus::Revoked {
            return Err(DistributionError::RevokedToolRelease {
                tool: package.tool_id,
                version: package.version,
            });
        }

        let _transaction = tool_state_guard(self.home)?;
        let mut tools = load_catalog_unlocked(self.home)?;
        let lock = ToolLock::from_package(&package);
        let active = InstalledTool::from_package(&package);
        let previous = tools.remove(&active.tool_id);
        let rollback = next_rollback(previous, &active);
        tools.insert(
            active.tool_id.clone(),
            ToolCatalogEntry {
                active: active.clone(),
                rollback,
            },
        );
        let stored = ToolCatalogFile {
            schema_version: TOOL_CATALOG_SCHEMA_VERSION,
            tools: tools.into_values().collect(),
        };
        commit_state_pair(
            &tool_lock_path(self.home, &active.tool_id),
            &encode_json(&lock)?,
            &self.home.tools_catalog_file(),
            &encode_json(&stored)?,
            writer,
        )?;
        Ok(active)
    }
}

fn next_rollback(previous: Option<ToolCatalogEntry>, active: &InstalledTool) -> Vec<InstalledTool> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let candidates = if previous.active == *active {
        previous.rollback
    } else {
        std::iter::once(previous.active)
            .chain(previous.rollback)
            .collect()
    };
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert((candidate.version.clone(), candidate.digest.clone())))
        .collect()
}

/// Read and validate the exact active lock for one tool.
pub fn read_tool_lock(home: &MorphirHome, id: &ToolId) -> Result<ToolLock> {
    let _transaction = tool_state_guard(home)?;
    read_tool_lock_unlocked(home, id)
}

fn read_tool_lock_unlocked(home: &MorphirHome, id: &ToolId) -> Result<ToolLock> {
    let path = tool_lock_path(home, id);
    let bytes = read_state_bytes(&path)?;
    let envelope: StateSchemaEnvelope = decode_state(&path, &bytes)?;
    if envelope.schema_version != TOOL_LOCK_SCHEMA_VERSION {
        return Err(DistributionError::UnsupportedStateSchema {
            kind: "tool lock",
            version: envelope.schema_version,
        });
    }
    let lock: ToolLock = decode_state(&path, &bytes)?;
    if &lock.tool_id != id {
        return Err(DistributionError::ToolStateMismatch { id: id.clone() });
    }
    Ok(lock)
}

/// Atomically list active tools with their exact selections and rollback releases.
pub fn list_installed_tools(home: &MorphirHome) -> Result<Vec<InstalledToolSnapshot>> {
    let _transaction = tool_state_guard(home)?;
    load_catalog_unlocked(home)?
        .into_values()
        .map(|entry| {
            let lock = read_tool_lock_unlocked(home, entry.active.tool_id())?;
            validate_active_pair(&entry.active, &lock)?;
            Ok(InstalledToolSnapshot {
                active: entry.active,
                rollback: entry.rollback,
                selection: lock.selection,
            })
        })
        .collect()
}

/// Offline launch contract whose active program bytes have just been reverified.
#[derive(Debug, Clone)]
pub struct VerifiedToolProcess {
    program: PathBuf,
    args: Vec<String>,
    tool_id: ToolId,
    version: Version,
}

impl VerifiedToolProcess {
    /// Return the verified absolute executable path.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Return fixed arguments prepended during launch.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Return the launched tool identity.
    pub fn tool_id(&self) -> &ToolId {
        &self.tool_id
    }

    /// Return the launched exact semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }
}

/// Resolve one active catalog entry without repository or network access and reverify its bytes.
pub fn activate_installed_tool(home: &MorphirHome, id: &ToolId) -> Result<VerifiedToolProcess> {
    let (active, lock) = {
        let _transaction = tool_state_guard(home)?;
        let tools = load_catalog_unlocked(home)?;
        let active = tools
            .get(id)
            .map(|entry| entry.active.clone())
            .ok_or_else(|| DistributionError::ToolNotInstalled { id: id.clone() })?;
        let lock = read_tool_lock_unlocked(home, id)?;
        (active, lock)
    };
    validate_active_pair(&active, &lock)?;
    let program = verify_installed(home, active.store_path.as_path(), &active.files)?;
    Ok(VerifiedToolProcess {
        program,
        args: active.args,
        tool_id: active.tool_id,
        version: active.version,
    })
}

fn load_catalog_unlocked(home: &MorphirHome) -> Result<BTreeMap<ToolId, ToolCatalogEntry>> {
    let path = home.tools_catalog_file();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let stored: ToolCatalogFile = read_json(&path)?;
    if stored.schema_version != TOOL_CATALOG_SCHEMA_VERSION {
        return Err(DistributionError::UnsupportedStateSchema {
            kind: "installed tool catalog",
            version: stored.schema_version,
        });
    }
    let mut tools = BTreeMap::new();
    for entry in stored.tools {
        let id = entry.active.tool_id.clone();
        if tools.insert(id.clone(), entry).is_some() {
            return Err(DistributionError::ToolStateMismatch { id });
        }
    }
    Ok(tools)
}

fn validate_active_pair(active: &InstalledTool, lock: &ToolLock) -> Result<()> {
    let matches = lock.schema_version == TOOL_LOCK_SCHEMA_VERSION
        && lock.tool_id == active.tool_id
        && lock.tool_name == active.tool_name
        && lock.version == active.version
        && lock.status == active.status
        && lock.platform == active.platform
        && lock.digest == active.digest
        && lock.length == active.length
        && lock.targets_version == active.targets_version
        && lock.target_path == active.target_path
        && lock.store_path == active.store_path
        && lock.args == active.args
        && lock.files == active.files;
    if matches {
        Ok(())
    } else {
        Err(DistributionError::ToolStateMismatch {
            id: active.tool_id.clone(),
        })
    }
}

fn verify_package(home: &MorphirHome, package: &VerifiedToolPackage) -> Result<PathBuf> {
    verify_installed(home, package.store_path.as_path(), &package.files)
}

fn verify_installed(
    home: &MorphirHome,
    store_path: &Path,
    files: &[ToolPackageFile],
) -> Result<PathBuf> {
    validate_manifest(store_path, files)?;
    let home_root = fs::canonicalize(home.root()).map_err(|source| DistributionError::Io {
        path: home.root().to_path_buf(),
        source,
    })?;
    let store_root =
        fs::canonicalize(home.tools_store_dir()).map_err(|source| DistributionError::Io {
            path: home.tools_store_dir(),
            source,
        })?;
    let requested = home.root().join(store_path);
    let program = fs::canonicalize(&requested).map_err(|source| DistributionError::Io {
        path: requested,
        source,
    })?;
    if !program.starts_with(&home_root) || !program.starts_with(&store_root) {
        return Err(DistributionError::InstalledPathEscape {
            path: program,
            root: home_root,
        });
    }
    for file in files {
        let path = home.root().join(file.path.as_path());
        let canonical =
            fs::canonicalize(&path).map_err(|source| DistributionError::Io { path, source })?;
        if !canonical.starts_with(&home_root) || !canonical.starts_with(&store_root) {
            return Err(DistributionError::InstalledPathEscape {
                path: canonical,
                root: home_root,
            });
        }
        verify_one_file(&canonical, &file.digest, file.length, file.executable)?;
    }
    Ok(program)
}

fn validate_manifest(store_path: &Path, files: &[ToolPackageFile]) -> Result<()> {
    if files.is_empty() {
        return Err(invalid_manifest("manifest cannot be empty"));
    }
    let unique = files.iter().map(|file| &file.path).collect::<BTreeSet<_>>();
    if unique.len() != files.len() {
        return Err(invalid_manifest("manifest paths must be unique"));
    }
    if !files
        .iter()
        .any(|file| file.path.as_path() == store_path && file.executable)
    {
        return Err(invalid_manifest(
            "manifest must contain the executable launch path",
        ));
    }
    Ok(())
}

fn invalid_manifest(reason: impl Into<String>) -> DistributionError {
    DistributionError::InvalidToolManifest {
        reason: reason.into(),
    }
}

fn verify_one_file(
    path: &Path,
    digest: &Sha256Digest,
    length: u64,
    executable: bool,
) -> Result<()> {
    verify_file(path, digest)?;
    let actual = fs::metadata(path)
        .map_err(|source| DistributionError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if actual != length {
        return Err(DistributionError::ToolLengthMismatch {
            path: path.to_path_buf(),
            expected: length,
            actual,
        });
    }
    verify_executable_mode(path, executable)
}

fn tool_state_guard(home: &MorphirHome) -> Result<StateGuard> {
    StateGuard::acquire(&home.tools_state_lock_file())
}

fn tool_lock_path(home: &MorphirHome, id: &ToolId) -> PathBuf {
    home.tools_locks_dir().join(format!("{id}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_io::{StateWriter, atomic_write_bytes};
    use crate::{
        Channel, Platform, RelativeArtifactPath, Selection, Sha256Digest, ToolId, ToolReleaseStatus,
    };
    use morphir_common::home::MorphirHome;
    use semver::Version;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    #[test]
    fn verified_tool_install_activates_offline_and_retains_rollback_release() {
        let root = tempfile::tempdir().unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let tool_id = ToolId::parse("desktop").unwrap();

        let first = package(&home, "1.0.0", b"desktop-v1");
        ToolInstaller::new(&home).install(first).unwrap();
        let launch = activate_installed_tool(&home, &tool_id).unwrap();
        assert_eq!(fs::read(launch.program()).unwrap(), b"desktop-v1");
        assert_eq!(launch.version(), &Version::parse("1.0.0").unwrap());

        let second = package(&home, "2.0.0", b"desktop-v2");
        ToolInstaller::new(&home).install(second).unwrap();
        let installed = list_installed_tools(&home).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(
            installed[0].active().version(),
            &Version::parse("2.0.0").unwrap()
        );
        assert_eq!(installed[0].rollback().len(), 1);
        assert_eq!(
            installed[0].rollback()[0].version(),
            &Version::parse("1.0.0").unwrap()
        );
        assert_eq!(
            installed[0].selection(),
            &Selection::Channel(Channel::Stable)
        );
    }

    #[test]
    fn authenticated_raw_download_is_reverified_and_published_before_activation() {
        let root = tempfile::tempdir().unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let download_root = root.path().join("downloads");
        fs::create_dir_all(&download_root).unwrap();
        let download = download_root.join("desktop.exe");
        let bytes = b"signed-desktop";
        fs::write(&download, bytes).unwrap();
        let digest = Sha256Digest::of_bytes(bytes);
        let release: crate::ToolReleaseRecord = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "kind": "morphir-tool-release",
            "tool": { "id": "desktop", "name": "Morphir Desktop" },
            "version": "1.0.0",
            "channels": ["stable"],
            "status": "active",
            "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
            "artifacts": [{
                "targetPath": "artifacts/desktop/1.0.0/desktop.exe",
                "platform": { "os": "windows", "arch": "x86_64" },
                "archive": { "format": "raw", "entryPoint": "desktop.exe" },
                "launch": {
                    "kind": "executable",
                    "path": "desktop.exe",
                    "args": ["--morphir-home"]
                }
            }]
        }))
        .unwrap();
        let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
            release,
            Selection::Channel(Channel::Stable),
            digest.clone(),
            bytes.len() as u64,
        );
        let downloaded = crate::DownloadedToolArtifact::test_fixture(download);

        let package = ToolPackageStore::new(&home)
            .prepare(resolved, downloaded)
            .unwrap();
        let installed = ToolInstaller::new(&home).install(package).unwrap();
        assert!(installed.store_path().starts_with("store/tools/sha256"));
        assert_eq!(installed.digest(), &digest);
        assert_eq!(
            activate_installed_tool(&home, &ToolId::parse("desktop").unwrap())
                .unwrap()
                .args(),
            ["--morphir-home"]
        );
    }

    #[test]
    fn authenticated_zip_is_atomically_expanded_and_every_file_is_reverified_offline() {
        let root = tempfile::tempdir().unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let download = root.path().join("desktop.zip");
        write_zip(
            &download,
            &[
                ("desktop.exe", b"signed-desktop"),
                ("resources/config.json", br#"{"ok":true}"#),
            ],
        );
        let bytes = fs::read(&download).unwrap();
        let digest = Sha256Digest::of_bytes(&bytes);
        let release = zip_release("desktop.exe");
        let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
            release,
            Selection::Channel(Channel::Stable),
            digest,
            bytes.len() as u64,
        );
        let downloaded = crate::DownloadedToolArtifact::test_fixture(download);

        let package = ToolPackageStore::new(&home)
            .prepare(resolved, downloaded)
            .unwrap();
        ToolInstaller::new(&home).install(package).unwrap();
        let id = ToolId::parse("desktop").unwrap();
        let launch = activate_installed_tool(&home, &id).unwrap();
        assert_eq!(fs::read(launch.program()).unwrap(), b"signed-desktop");

        let support_file = launch
            .program()
            .parent()
            .unwrap()
            .join("resources/config.json");
        fs::write(support_file, b"tampered").unwrap();
        assert!(matches!(
            activate_installed_tool(&home, &id).unwrap_err(),
            crate::DistributionError::DigestMismatch { .. }
        ));
    }

    #[test]
    fn zip_traversal_is_rejected_without_publishing_or_activating_a_tool() {
        let root = tempfile::tempdir().unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let download = root.path().join("desktop.zip");
        write_zip(&download, &[("../escape.exe", b"escape")]);
        let bytes = fs::read(&download).unwrap();
        let resolved = crate::ResolvedTrustedToolArtifact::test_fixture(
            zip_release("desktop.exe"),
            Selection::Channel(Channel::Stable),
            Sha256Digest::of_bytes(&bytes),
            bytes.len() as u64,
        );

        let error = ToolPackageStore::new(&home)
            .prepare(
                resolved,
                crate::DownloadedToolArtifact::test_fixture(download),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            crate::DistributionError::UnsafeToolArchive { .. }
        ));
        assert!(!root.path().join("escape.exe").exists());
        assert!(!home.tools_catalog_file().exists());
    }

    #[test]
    fn failed_tool_catalog_commit_restores_the_previous_active_release() {
        let root = tempfile::tempdir().unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let tool_id = ToolId::parse("desktop").unwrap();
        ToolInstaller::new(&home)
            .install(package(&home, "1.0.0", b"desktop-v1"))
            .unwrap();

        let writer = FailingCatalogWriter {
            catalog_path: home.tools_catalog_file(),
        };
        let error = ToolInstaller::new(&home)
            .install_with_writer(package(&home, "2.0.0", b"desktop-v2"), &writer)
            .unwrap_err();
        assert!(error.to_string().contains("injected tool catalog failure"));

        let launch = activate_installed_tool(&home, &tool_id).unwrap();
        assert_eq!(launch.version(), &Version::parse("1.0.0").unwrap());
        assert_eq!(fs::read(launch.program()).unwrap(), b"desktop-v1");
    }

    #[test]
    fn tool_install_rejects_a_manifest_that_does_not_cover_the_launch_program() {
        let root = tempfile::tempdir().unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let mut candidate = package(&home, "1.0.0", b"desktop");
        candidate.files.clear();

        assert!(matches!(
            ToolInstaller::new(&home).install(candidate).unwrap_err(),
            crate::DistributionError::InvalidToolManifest { .. }
        ));
        assert!(!home.tools_catalog_file().exists());
    }

    struct FailingCatalogWriter {
        catalog_path: PathBuf,
    }

    impl StateWriter for FailingCatalogWriter {
        fn write(&self, path: &Path, bytes: &[u8]) -> crate::Result<()> {
            atomic_write_bytes(path, bytes)?;
            if path == self.catalog_path {
                return Err(crate::DistributionError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("injected tool catalog failure"),
                });
            }
            Ok(())
        }
    }

    fn package(home: &MorphirHome, version: &str, bytes: &[u8]) -> VerifiedToolPackage {
        let digest = Sha256Digest::of_bytes(bytes);
        let relative =
            RelativeArtifactPath::parse(format!("store/tools/sha256/{digest}/desktop.exe"))
                .unwrap();
        let path = home.root().join(relative.as_path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let file = ToolPackageFile {
            path: relative.clone(),
            digest: digest.clone(),
            length: bytes.len() as u64,
            executable: true,
        };
        VerifiedToolPackage {
            selection: Selection::Channel(Channel::Stable),
            tool_id: ToolId::parse("desktop").unwrap(),
            tool_name: "Morphir Desktop".to_owned(),
            version: Version::parse(version).unwrap(),
            status: ToolReleaseStatus::Active,
            platform: Platform::new("windows", "x86_64").unwrap(),
            digest,
            length: bytes.len() as u64,
            targets_version: 1,
            target_path: RelativeArtifactPath::parse(format!(
                "artifacts/desktop/{version}/desktop.exe"
            ))
            .unwrap(),
            store_path: relative,
            args: vec!["--morphir-home".to_owned()],
            files: vec![file],
        }
    }

    fn zip_release(entry_point: &str) -> crate::ToolReleaseRecord {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "kind": "morphir-tool-release",
            "tool": { "id": "desktop", "name": "Morphir Desktop" },
            "version": "1.0.0",
            "channels": ["stable"],
            "status": "active",
            "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
            "artifacts": [{
                "targetPath": "artifacts/desktop/1.0.0/desktop.zip",
                "platform": { "os": "windows", "arch": "x86_64" },
                "archive": { "format": "zip", "entryPoint": entry_point },
                "launch": {
                    "kind": "executable",
                    "path": entry_point,
                    "args": ["--morphir-home"]
                }
            }]
        }))
        .unwrap()
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
    }
}
