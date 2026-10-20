//! Bounded observations of actual native configuration reads.
//!
//! A sealed complete snapshot covers the instrumented synchronous SDK prefix
//! only. It proves neither whole-host coverage nor worker completion, and grants
//! no filesystem authority. Ordinary uninstrumented APIs produce no evidence.
//!
//! ```no_run
//! use cap_std::{ambient_authority, fs::Dir};
//! use morphir_common::config::{load_config_value_observed, observation::{ReadLimits, SourceReadScope}};
//! use std::path::Path;
//! # fn main() -> anyhow::Result<()> {
//! let root = Path::new("/already-authorized/workspace");
//! let held = Dir::open_ambient_dir(root, ambient_authority())?;
//! // Illustrative caller-issued evidence limits, not a production default.
//! let limits = ReadLimits::new(16, 128, 1_048_576, 1)?;
//! let scope = SourceReadScope::bind_workspace(&held, root, limits)?;
//! let result = load_config_value_observed(&root.join("morphir.toml"), &scope);
//! let evidence = scope.seal();
//! evidence.require_complete()?;
//! result?;
//! # Ok(()) }
//! ```
use cap_std::fs::{Dir, OpenOptions};
mod identity;
use identity::FileIdentity;
use std::{
    fs::File,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

/// Why bounded observation evidence cannot establish a complete prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ObservationError {
    /// The supplied label does not identify the held workspace directory.
    #[error("workspace observation identity does not match")]
    RootIdentity,
    /// A registered path must be relative to its supplied directory.
    #[error("observation file path is not relative")]
    InvalidPath,
    /// A real file's identity could not be established.
    #[error("observation file identity is unavailable")]
    FileIdentity,
    /// Retained observations exhausted their caller-issued bound.
    #[error("observation capacity exceeded")]
    Capacity,
    /// This witness belongs to another observation owner.
    #[error("foreign observation file witness")]
    ForeignWitness,
    /// File witnesses must precede every actual content attempt.
    #[error("observation file registration is closed")]
    RegistrationClosed,
    /// Missing, unfinished or uncertain observations cannot certify zero.
    #[error("native SDK read prefix is not completely observed")]
    Incomplete,
}
/// Caller-issued finite observation bounds, separate from source-read quotas.
#[derive(Debug, Clone, Copy)]
pub struct ReadLimits {
    attempts: u64,
    calls: u64,
    bytes: u64,
    files: usize,
}
impl ReadLimits {
    /// Zero bounds are valid and make any corresponding activity incomplete.
    pub fn new(
        content_attempts: u64,
        read_calls: u64,
        returned_bytes: u64,
        registered_files: usize,
    ) -> Result<Self, ObservationError> {
        if registered_files > isize::MAX as usize / std::mem::size_of::<RegisteredFile>() {
            return Err(ObservationError::Capacity);
        }
        Ok(Self {
            attempts: content_attempts,
            calls: read_calls,
            bytes: returned_bytes,
            files: registered_files,
        })
    }
}
/// Actual content location relative to the held selected workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadClass {
    /// Acquired through a held directory grant proved under the selected root.
    WorkspaceContent,
    /// An outside open route, even if separately permitted or targeting an
    /// in-root file identity through an alias.
    AmbientContent,
}
impl ReadClass {
    fn index(self) -> usize {
        match self {
            Self::WorkspaceContent => 0,
            Self::AmbientContent => 1,
        }
    }
}
/// Coverage of this measured SDK prefix, not execution success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// No instrumented content route was entered.
    Unobserved,
    /// Unsealed, active, overflowed or uncertain evidence.
    Partial,
    /// A sealed, fully observed synchronous SDK prefix.
    Complete,
}
/// Fixed counters; returned bytes include bytes rejected by later UTF-8 parsing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReadCounts {
    attempts: u64,
    calls: u64,
    bytes: u64,
}
impl ReadCounts {
    /// Actual content-open/read attempts, not metadata registration.
    pub fn content_attempts(&self) -> u64 {
        self.attempts
    }
    /// Actual calls to the underlying file's Read implementation, including EOF.
    pub fn read_calls(&self) -> u64 {
        self.calls
    }
    /// Bytes actually returned by those calls.
    pub fn returned_bytes(&self) -> u64 {
        self.bytes
    }
}
#[derive(Debug)]
struct RegisteredFile {
    identity: FileIdentity,
    counts: ReadCounts,
}
#[derive(Debug, Clone, Copy)]
enum Phase {
    Registering,
    Reading,
    Sealed,
}
#[derive(Debug)]
struct State {
    phase: Phase,
    fault: Option<ObservationError>,
    active: usize,
    total: ReadCounts,
    classes: [ReadCounts; 2],
    files: Vec<RegisteredFile>,
}
#[derive(Debug)]
struct Shared {
    limits: ReadLimits,
    state: Mutex<State>,
}
impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poison| {
            let mut state = poison.into_inner();
            state.fault.get_or_insert(ObservationError::Incomplete);
            state
        })
    }
}
/// Opaque registration of an actual file identity under one issued scope.
pub struct FileReadWitness {
    owner: Arc<Shared>,
    index: usize,
}
/// Non-Clone writer owner bound before the observed native invocation.
#[derive(Debug)]
pub struct SourceReadScope {
    directory: Dir,
    issued_label: PathBuf,
    label: PathBuf,
    identity: FileIdentity,
    shared: Arc<Shared>,
}
/// Cloneable read-only view. It exposes no event setters or directory grants.
#[derive(Clone)]
pub struct ReadObservation {
    shared: Arc<Shared>,
}
/// Immutable bounded evidence. Its private fields cannot issue host authority.
pub struct ReadSnapshot {
    owner: Arc<Shared>,
    coverage: Coverage,
    classes: [ReadCounts; 2],
    total: ReadCounts,
    files: Vec<ReadCounts>,
}
impl ReadSnapshot {
    /// Current SDK-prefix coverage.
    pub fn coverage(&self) -> Coverage {
        self.coverage
    }
    /// Refuse all zero assertions unless this SDK prefix was completely observed.
    pub fn require_complete(&self) -> Result<(), ObservationError> {
        if self.coverage == Coverage::Complete {
            Ok(())
        } else {
            Err(ObservationError::Incomplete)
        }
    }
    /// Actual counts for this location class.
    pub fn counts(&self, class: ReadClass) -> ReadCounts {
        self.classes[class.index()]
    }
    /// All retained actual attempts, including unclassified or failed opens.
    /// Overflow can truncate this prefix; require complete coverage before
    /// interpreting a zero as measured absence.
    pub fn total_counts(&self) -> ReadCounts {
        self.total
    }
    /// Query a same-issued-scope actual file identity; names grant no identity.
    pub fn counts_for(&self, witness: &FileReadWitness) -> Result<ReadCounts, ObservationError> {
        if !Arc::ptr_eq(&self.owner, &witness.owner) {
            return Err(ObservationError::ForeignWitness);
        }
        self.files
            .get(witness.index)
            .copied()
            .ok_or(ObservationError::ForeignWitness)
    }
}
fn snapshot(shared: &Arc<Shared>) -> ReadSnapshot {
    let mut state = shared.state();
    let mut files = Vec::new();
    if files.try_reserve_exact(state.files.len()).is_err() {
        state.fault.get_or_insert(ObservationError::Capacity);
    } else {
        files.extend(state.files.iter().map(|file| file.counts));
    }
    let coverage = if state.fault.is_some() || state.active != 0 {
        Coverage::Partial
    } else if state.total.attempts == 0 {
        Coverage::Unobserved
    } else {
        match state.phase {
            Phase::Registering | Phase::Reading => Coverage::Partial,
            Phase::Sealed => Coverage::Complete,
        }
    };
    ReadSnapshot {
        owner: shared.clone(),
        coverage,
        classes: state.classes,
        total: state.total,
        files,
    }
}
impl ReadObservation {
    /// Snapshotting before owner seal cannot certify completeness.
    pub fn snapshot(&self) -> ReadSnapshot {
        snapshot(&self.shared)
    }
}
impl SourceReadScope {
    /// Execute a real capability read. Classification is complete only when the
    /// supplied directory is the same held workspace root; no label issues a grant.
    pub fn read_capability(
        &self,
        directory: &Dir,
        relative: &Path,
        limit: u64,
    ) -> io::Result<Vec<u8>> {
        let reader = self.content_directory(directory, Some(&self.issued_label));
        let mut options = OpenOptions::new();
        options.read(true);
        read_bytes(reader.open_content_file(relative, &options)?, limit)
    }
    /// Borrow an existing directory grant for real content opens. The optional
    /// label supplies identity evidence, never authority. A missing, changed or
    /// unprovable binding leaves subsequent real reads unclassified/Partial.
    /// The temporary label is not borrowed by the returned wrapper.
    /// Workspace evidence requires the actual selected root as the opening
    /// directory. A proper descendant can move during an open, so metadata
    /// checks cannot certify its acquisition as workspace content. Such reads
    /// preserve the caller's operation and remain Partial.
    pub fn content_directory<'scope, 'directory>(
        &'scope self,
        held: &'directory Dir,
        native_label: Option<&Path>,
    ) -> ContentDirectory<'scope, 'directory> {
        let provenance = native_label
            .and_then(|label| std::path::absolute(label).ok())
            .map(|label| {
                let location = self.directory_location(held, &label);
                DirectoryProvenance::Bound { label, location }
            })
            .unwrap_or(DirectoryProvenance::Unclassified);
        ContentDirectory {
            scope: self,
            directory: held,
            provenance,
            #[cfg(test)]
            after_data_open: None,
        }
    }
    /// Bind identity metadata only. No file content is read and no new authority
    /// is exposed. The caller already owns both the directory and native access.
    pub fn bind_workspace(
        held: &Dir,
        label: &Path,
        limits: ReadLimits,
    ) -> Result<Self, ObservationError> {
        let identity = FileIdentity::from_file(
            held.try_clone()
                .map_err(|_| ObservationError::RootIdentity)?
                .into_std_file(),
        )
        .map_err(|_| ObservationError::RootIdentity)?;
        let issued_label =
            std::path::absolute(label).map_err(|_| ObservationError::RootIdentity)?;
        let canonical =
            std::fs::canonicalize(&issued_label).map_err(|_| ObservationError::RootIdentity)?;
        if identity
            != FileIdentity::from_path(&canonical).map_err(|_| ObservationError::RootIdentity)?
        {
            return Err(ObservationError::RootIdentity);
        }
        Ok(Self {
            directory: held
                .try_clone()
                .map_err(|_| ObservationError::RootIdentity)?,
            label: canonical,
            issued_label,
            identity,
            shared: Arc::new(Shared {
                limits,
                state: Mutex::new(State {
                    phase: Phase::Registering,
                    fault: None,
                    active: 0,
                    total: ReadCounts::default(),
                    classes: [ReadCounts::default(); 2],
                    files: Vec::new(),
                }),
            }),
        })
    }
    /// Retain a read-only view of the same owner.
    pub fn observation(&self) -> ReadObservation {
        ReadObservation {
            shared: self.shared.clone(),
        }
    }
    /// Register actual file identity using metadata only, with bounded retention.
    pub fn register_file(
        &self,
        held: &Dir,
        relative: &Path,
    ) -> Result<FileReadWitness, ObservationError> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(ObservationError::InvalidPath);
        }
        // The same lock closes registration before ANY first attempt, including
        // failed opens and attempts omitted by the counter bound. Holding it
        // through metadata identity issuance prevents concurrent retroactive zeros.
        let mut state = self.shared.state();
        if !matches!(state.phase, Phase::Registering) {
            return Err(ObservationError::RegistrationClosed);
        }
        let file = held
            .open(relative)
            .map_err(|_| ObservationError::FileIdentity)?;
        let identity =
            FileIdentity::from_file(file.into_std()).map_err(|_| ObservationError::FileIdentity)?;
        if let Some(index) = state
            .files
            .iter()
            .position(|entry| entry.identity == identity)
        {
            return Ok(FileReadWitness {
                owner: self.shared.clone(),
                index,
            });
        }
        if state.files.len() == self.shared.limits.files || state.files.try_reserve(1).is_err() {
            state.fault.get_or_insert(ObservationError::Capacity);
            return Err(ObservationError::Capacity);
        }
        let index = state.files.len();
        state.files.push(RegisteredFile {
            identity,
            counts: ReadCounts::default(),
        });
        Ok(FileReadWitness {
            owner: self.shared.clone(),
            index,
        })
    }
    /// Seal only the already performed synchronous SDK prefix. This is not a
    /// whole-SDK/native-host/capture/all-workers-exited certificate.
    pub fn seal(self) -> ReadSnapshot {
        self.shared.state().phase = Phase::Sealed;
        snapshot(&self.shared)
    }
    fn root_is_current(&self) -> bool {
        [&self.issued_label, &self.label].into_iter().all(|label| {
            FileIdentity::from_path(label)
                .ok()
                .is_some_and(|identity| identity == self.identity)
        })
    }
    fn relative_route<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        if !path.is_absolute() {
            return None;
        }
        path.strip_prefix(&self.issued_label)
            .ok()
            .or_else(|| path.strip_prefix(&self.label).ok())
    }
    fn directory_location(&self, held: &Dir, label: &Path) -> DirectoryLocation {
        if !self.root_is_current() {
            return DirectoryLocation::Unclassified;
        }
        let Some(identity) = directory_identity(held) else {
            return DirectoryLocation::Unclassified;
        };
        if FileIdentity::from_path(label).ok().as_ref() != Some(&identity) {
            return DirectoryLocation::Unclassified;
        }
        let Some(relative) = self.relative_route(label) else {
            return DirectoryLocation::Ambient;
        };
        // Only this actual root grant makes the data open root-confined.
        // Reopening a descendant for metadata before/after data acquisition
        // cannot exclude a move outside and back between those observations.
        if relative.as_os_str().is_empty() && identity == self.identity {
            DirectoryLocation::SelectedRoot
        } else {
            let outside = std::fs::canonicalize(label)
                .ok()
                .is_some_and(|target| !target.starts_with(&self.label));
            if outside {
                DirectoryLocation::Ambient
            } else {
                DirectoryLocation::Unclassified
            }
        }
    }
    fn native_class(&self, file: &File, path: &Path) -> Option<ReadClass> {
        if !self.root_is_current() || !path.is_absolute() {
            return None;
        }
        let actual = FileIdentity::from_file(file.try_clone().ok()?).ok()?;
        if actual != FileIdentity::from_path(path).ok()? {
            return None;
        }
        // Native fallback can establish Ambient, never Workspace: the latter
        // requires the actual data descriptor to come from the held grant.
        if self.relative_route(path).is_none() {
            return Some(ReadClass::AmbientContent);
        }
        let target = std::fs::canonicalize(path).ok()?;
        (!target.starts_with(&self.label)).then_some(ReadClass::AmbientContent)
    }
}
fn directory_identity(directory: &Dir) -> Option<FileIdentity> {
    FileIdentity::from_file(directory.try_clone().ok()?.into_std_file()).ok()
}
#[derive(Clone, Copy)]
enum DirectoryLocation {
    SelectedRoot,
    Ambient,
    Unclassified,
}
enum DirectoryProvenance {
    Bound {
        label: PathBuf,
        location: DirectoryLocation,
    },
    Unclassified,
}
/// Opaque borrow of a caller-owned real directory. It exposes neither the grant
/// nor an event recorder, and cannot be constructed from a reader or byte vector.
pub struct ContentDirectory<'scope, 'directory> {
    scope: &'scope SourceReadScope,
    directory: &'directory Dir,
    provenance: DirectoryProvenance,
    #[cfg(test)]
    after_data_open: Option<&'scope dyn Fn()>,
}
impl<'scope> ContentDirectory<'scope, '_> {
    /// Perform the actual open using unchanged caller options, including nofollow
    /// and nonblocking. Opening and metadata inspection do not read eagerly.
    ///
    /// ```no_run
    /// use cap_std::{ambient_authority, fs::{Dir, OpenOptions}};
    /// use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
    /// use morphir_common::config::observation::{ReadLimits, SourceReadScope};
    /// use std::{io::Read as _, path::Path};
    /// # fn main() -> anyhow::Result<()> {
    /// let path = Path::new("/already-authorized/workspace");
    /// let held = Dir::open_ambient_dir(path, ambient_authority())?;
    /// let scope = SourceReadScope::bind_workspace(&held, path, ReadLimits::new(16, 128, 1_048_576, 1)?)?;
    /// let directory = scope.content_directory(&held, Some(path));
    /// let mut options = OpenOptions::new();
    /// options.read(true).follow(FollowSymlinks::No);
    /// let mut file = directory.open_content_file(Path::new("model.json"), &options)?;
    /// let mut prefix = [0; 7];
    /// file.read_exact(&mut prefix)?;
    /// drop(file);
    /// drop(directory);
    /// scope.seal().require_complete()?;
    /// # Ok(()) }
    /// ```
    pub fn open_content_file(
        &self,
        relative: &Path,
        options: &OpenOptions,
    ) -> io::Result<ObservedContentFile<'scope>> {
        let mut attempt = Attempt::begin(self.scope);
        let file = self.directory.open_with(relative, options)?;
        #[cfg(test)]
        if let Some(after_data_open) = self.after_data_open {
            after_data_open();
        }
        let class = match &self.provenance {
            DirectoryProvenance::Bound { label, location } => {
                let current = self.scope.directory_location(self.directory, label);
                match (location, current) {
                    (DirectoryLocation::SelectedRoot, DirectoryLocation::SelectedRoot) => {
                        Some(ReadClass::WorkspaceContent)
                    }
                    (DirectoryLocation::Ambient, DirectoryLocation::Ambient) => {
                        Some(ReadClass::AmbientContent)
                    }
                    _ => None,
                }
            }
            DirectoryProvenance::Unclassified => None,
        };
        attempt.opened(&file, class);
        Ok(ObservedContentFile { file, attempt })
    }
}
struct Attempt<'a> {
    scope: &'a SourceReadScope,
    class: Option<ReadClass>,
    file: Option<usize>,
    recording: AttemptRecording,
}
enum AttemptRecording {
    Recorded,
    Omitted,
}
enum Increment {
    ReadCall,
    ReturnedBytes(u64),
}
impl<'a> Attempt<'a> {
    fn begin(scope: &'a SourceReadScope) -> Self {
        let mut state = scope.shared.state();
        state.phase = Phase::Reading;
        match state.active.checked_add(1) {
            Some(active) => state.active = active,
            None => {
                state.fault.get_or_insert(ObservationError::Capacity);
            }
        }
        let recording = match state
            .total
            .attempts
            .checked_add(1)
            .filter(|count| *count <= scope.shared.limits.attempts)
        {
            Some(count) => {
                state.total.attempts = count;
                AttemptRecording::Recorded
            }
            None => {
                state.fault.get_or_insert(ObservationError::Capacity);
                AttemptRecording::Omitted
            }
        };
        Self {
            scope,
            class: None,
            file: None,
            recording,
        }
    }
    fn opened(&mut self, file: &cap_std::fs::File, class: Option<ReadClass>) {
        self.class = class;
        let identity = file
            .try_clone()
            .ok()
            .and_then(|file| FileIdentity::from_file(file.into_std()).ok());
        let mut state = self.scope.shared.state();
        if self.class.is_none() || identity.is_none() {
            state.fault.get_or_insert(ObservationError::FileIdentity);
        }
        if matches!(self.recording, AttemptRecording::Omitted) {
            return;
        }
        if let Some(class) = self.class {
            state.classes[class.index()].attempts += 1;
        }
        if let Some(identity) = identity {
            // Registration reuses existing identities, so at most one bounded
            // registry slot can match an opened file, including hardlink aliases.
            if let Some(index) = state
                .files
                .iter()
                .position(|entry| entry.identity == identity)
            {
                state.files[index].counts.attempts += 1;
                self.file = Some(index);
            }
        }
    }
    fn call(&self) {
        self.charge(Increment::ReadCall);
    }
    fn returned(&self, bytes: usize) {
        match u64::try_from(bytes) {
            Ok(bytes) => self.charge(Increment::ReturnedBytes(bytes)),
            Err(_) => {
                self.scope
                    .shared
                    .state()
                    .fault
                    .get_or_insert(ObservationError::Capacity);
            }
        }
    }
    fn charge(&self, increment: Increment) {
        let mut state = self.scope.shared.state();
        let next = match increment {
            Increment::ReadCall => state
                .total
                .calls
                .checked_add(1)
                .filter(|n| *n <= self.scope.shared.limits.calls),
            Increment::ReturnedBytes(bytes) => state
                .total
                .bytes
                .checked_add(bytes)
                .filter(|n| *n <= self.scope.shared.limits.bytes),
        };
        let Some(next) = next else {
            state.fault.get_or_insert(ObservationError::Capacity);
            return;
        };
        match increment {
            Increment::ReadCall => state.total.calls = next,
            Increment::ReturnedBytes(_) => state.total.bytes = next,
        }
        if matches!(self.recording, AttemptRecording::Omitted) {
            return;
        }
        let add = |counts: &mut ReadCounts| match increment {
            Increment::ReadCall => counts.calls += 1,
            Increment::ReturnedBytes(bytes) => counts.bytes += bytes,
        };
        if let Some(class) = self.class {
            add(&mut state.classes[class.index()]);
        }
        if let Some(index) = self.file {
            add(&mut state.files[index].counts);
        }
    }
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        let mut state = self.scope.shared.state();
        state.active = state.active.saturating_sub(1);
        if self.class.is_none() {
            state.fault.get_or_insert(ObservationError::FileIdentity);
        }
    }
}
/// A real opened content file. Only actual I/O entrypoints construct it; callers
/// cannot inject a synthetic Read implementation or observation events.
pub struct ObservedContentFile<'a> {
    file: cap_std::fs::File,
    attempt: Attempt<'a>,
}
impl ObservedContentFile<'_> {
    /// Inspect actual file metadata without generating a content read call.
    pub fn metadata(&self) -> io::Result<cap_std::fs::Metadata> {
        self.file.metadata()
    }
}
impl Read for ObservedContentFile<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.attempt.call();
        let count = self.file.read(bytes)?;
        self.attempt.returned(count);
        Ok(count)
    }
}
fn read_bytes(file: ObservedContentFile<'_>, limit: u64) -> io::Result<Vec<u8>> {
    let mut reader = file.take(limit);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}
/// Perform an actual native read under existing caller authority, preserving a
/// bounded read limit. No peer can supply counters or synthetic read results.
pub fn read_native_file(scope: &SourceReadScope, path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    if scope.root_is_current()
        && let Some(relative) = scope.relative_route(path)
    {
        let directory = scope.content_directory(&scope.directory, Some(&scope.issued_label));
        let mut options = OpenOptions::new();
        options.read(true);
        if let Ok(file) = directory.open_content_file(relative, &options) {
            return read_bytes(file, limit);
        }
        // This was a REAL failed content open, not a metadata probe. Its first
        // attempt is retained/Partial; native fallback is a second real attempt.
    }
    let mut attempt = Attempt::begin(scope);
    let file = File::open(path)?;
    let class = scope.native_class(&file, path);
    let file = cap_std::fs::File::from_std(file);
    attempt.opened(&file, class);
    read_bytes(ObservedContentFile { file, attempt }, limit)
}

#[cfg(test)]
mod acquisition_boundary_tests;
