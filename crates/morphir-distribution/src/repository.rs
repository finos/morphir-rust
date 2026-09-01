//! Named extension repository configuration and lifecycle operations.

use crate::domain::portable_token;
use crate::state_io::{StateGuard, atomic_write_json, decode_state, read_state_bytes};
use crate::{
    DistributionError, ExtensionHistory, ExtensionId, LocalIndex, Platform, ResolvedArtifact,
    Result, Selection,
};
use morphir_common::home::MorphirHome;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const REPOSITORIES_SCHEMA_VERSION: u32 = 1;
const MAX_REPOSITORY_NAME_LEN: usize = 64;

/// A stable lowercase name for one configured extension repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryName(String);

impl RepositoryName {
    /// Parse a portable repository name such as `local-dev`.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() <= MAX_REPOSITORY_NAME_LEN && portable_token(&value) {
            Ok(Self(value))
        } else {
            Err(crate::error::invalid_value(
                "repository name",
                value,
                "expected a lowercase portable token beginning with a letter and at most 64 characters",
            ))
        }
    }

    /// Return the portable repository name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for RepositoryName {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RepositoryName {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Access location for an extension repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryEndpoint(RepositoryEndpointKind);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum RepositoryEndpointKind {
    LocalDirectory { path: PathBuf },
}

impl RepositoryEndpoint {
    /// Validate and canonicalize an existing local repository directory.
    pub fn local_directory(path: impl AsRef<Path>) -> Result<Self> {
        let index = LocalIndex::open(path)?;
        let metadata_directory = index.root().join("extensions");
        if !metadata_directory.is_dir() {
            return Err(DistributionError::Io {
                path: metadata_directory,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "local extension repository has no extensions metadata directory",
                ),
            });
        }
        Ok(Self(RepositoryEndpointKind::LocalDirectory {
            path: index.root().to_path_buf(),
        }))
    }

    /// Return the stable endpoint-kind spelling.
    pub fn kind(&self) -> &'static str {
        match &self.0 {
            RepositoryEndpointKind::LocalDirectory { .. } => "local-directory",
        }
    }

    /// Return the local directory path when this is a local endpoint.
    pub fn local_directory_path(&self) -> Option<&Path> {
        match &self.0 {
            RepositoryEndpointKind::LocalDirectory { path } => Some(path),
        }
    }

    fn local_index(&self) -> Result<LocalIndex> {
        match &self.0 {
            RepositoryEndpointKind::LocalDirectory { path } => LocalIndex::open(path),
        }
    }

    fn validated(self) -> Result<Self> {
        match self.0 {
            RepositoryEndpointKind::LocalDirectory { path } => Self::local_directory(path),
        }
    }
}

/// Whether a configured repository participates in extension resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepositoryState {
    /// The repository can resolve extensions.
    Enabled,
    /// The repository remains configured but cannot resolve extensions.
    Disabled,
}

impl RepositoryState {
    /// Return the stable serialized state spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

/// One named extension repository configured in Morphir Home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionRepository {
    name: RepositoryName,
    endpoint: RepositoryEndpoint,
    state: RepositoryState,
}

impl ExtensionRepository {
    /// Return the repository name.
    pub fn name(&self) -> &RepositoryName {
        &self.name
    }

    /// Return the configured endpoint.
    pub fn endpoint(&self) -> &RepositoryEndpoint {
        &self.endpoint
    }

    /// Return whether the repository is enabled.
    pub fn state(&self) -> RepositoryState {
        self.state
    }
}

/// A non-empty case-insensitive query over extension identity and display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionSearchQuery {
    normalized: String,
}

impl ExtensionSearchQuery {
    /// Parse a search query after trimming surrounding whitespace.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(crate::error::invalid_value(
                "extension search query",
                value,
                "expected non-whitespace identity or text",
            ));
        }
        Ok(Self {
            normalized: normalize_search_text(trimmed),
        })
    }

    fn matches(&self, release: &crate::ReleaseRecord) -> bool {
        normalize_search_text(release.extension_id().as_str()).contains(&self.normalized)
            || normalize_search_text(release.name()).contains(&self.normalized)
    }
}

/// One release found in an enabled repository, including its exact provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionSearchResult {
    repository: ExtensionRepository,
    release: crate::ReleaseRecord,
}

impl ExtensionSearchResult {
    /// Return the configured repository that supplied this result.
    pub fn repository(&self) -> &ExtensionRepository {
        &self.repository
    }

    /// Return the exact matching release record.
    pub fn release(&self) -> &crate::ReleaseRecord {
        &self.release
    }
}

/// Counts produced by validating repository metadata without installing bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryVerification {
    history_count: usize,
    release_count: usize,
}

impl RepositoryVerification {
    /// Return the number of validated extension history files.
    pub fn history_count(self) -> usize {
        self.history_count
    }

    /// Return the number of validated release records.
    pub fn release_count(self) -> usize {
        self.release_count
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryFile {
    schema_version: u32,
    repositories: Vec<ExtensionRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateSchemaEnvelope {
    schema_version: u32,
}

/// Locked lifecycle operations for extension repositories in one Morphir Home.
///
/// ```
/// use morphir_common::home::MorphirHome;
/// use morphir_distribution::{
///     ExtensionRepositories, RepositoryEndpoint, RepositoryName, RepositoryState,
/// };
/// use std::fs;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let scratch = tempfile::tempdir()?;
/// # let home_path = scratch.path().join("home");
/// # let repository_path = scratch.path().join("repository");
/// # fs::create_dir_all(repository_path.join("extensions"))?;
/// # let home = MorphirHome::resolve_from(Some(home_path.as_os_str()), None)?;
/// let repositories = ExtensionRepositories::new(&home);
/// let name = RepositoryName::parse("local-dev")?;
/// let endpoint = RepositoryEndpoint::local_directory(&repository_path)?;
///
/// let added = repositories.add(name.clone(), endpoint)?;
/// assert_eq!(added.state(), RepositoryState::Enabled);
/// assert_eq!(repositories.disable(&name)?.state(), RepositoryState::Disabled);
/// assert_eq!(repositories.enable(&name)?.state(), RepositoryState::Enabled);
/// assert_eq!(repositories.remove(&name)?.name(), &name);
/// # Ok(())
/// # }
/// # example().unwrap();
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ExtensionRepositories<'home> {
    home: &'home MorphirHome,
}

impl<'home> ExtensionRepositories<'home> {
    /// Bind repository operations to a Morphir Home.
    pub fn new(home: &'home MorphirHome) -> Self {
        Self { home }
    }

    /// Add an enabled repository without replacing an existing name.
    pub fn add(
        &self,
        name: RepositoryName,
        endpoint: RepositoryEndpoint,
    ) -> Result<ExtensionRepository> {
        let endpoint = endpoint.validated()?;
        let _guard = self.acquire()?;
        let mut repositories = self.load_unlocked()?;
        if repositories.contains_key(&name) {
            return Err(DistributionError::RepositoryAlreadyExists { name });
        }
        let repository = ExtensionRepository {
            name: name.clone(),
            endpoint,
            state: RepositoryState::Enabled,
        };
        repositories.insert(name, repository.clone());
        self.persist(&repositories)?;
        tracing::info!(
            event_name = "extension.repository.add",
            repository = %repository.name,
            endpoint_kind = repository.endpoint.kind(),
            state = repository.state.as_str(),
            "extension repository added"
        );
        Ok(repository)
    }

    /// List configured repositories in stable name order without contacting them.
    pub fn list(&self) -> Result<Vec<ExtensionRepository>> {
        let _guard = self.acquire()?;
        Ok(self.load_unlocked()?.into_values().collect())
    }

    /// Read one configured repository without contacting its endpoint.
    pub fn get(&self, name: &RepositoryName) -> Result<ExtensionRepository> {
        let _guard = self.acquire()?;
        self.load_unlocked()?
            .remove(name)
            .ok_or_else(|| DistributionError::RepositoryNotFound { name: name.clone() })
    }

    /// Enable a configured repository.
    pub fn enable(&self, name: &RepositoryName) -> Result<ExtensionRepository> {
        self.set_state(name, RepositoryState::Enabled)
    }

    /// Disable a configured repository while retaining its configuration.
    pub fn disable(&self, name: &RepositoryName) -> Result<ExtensionRepository> {
        self.set_state(name, RepositoryState::Disabled)
    }

    /// Remove a configured repository without deleting endpoint content.
    pub fn remove(&self, name: &RepositoryName) -> Result<ExtensionRepository> {
        let _guard = self.acquire()?;
        let mut repositories = self.load_unlocked()?;
        let removed = repositories
            .remove(name)
            .ok_or_else(|| DistributionError::RepositoryNotFound { name: name.clone() })?;
        self.persist(&repositories)?;
        tracing::info!(
            event_name = "extension.repository.remove",
            repository = %removed.name,
            endpoint_kind = removed.endpoint.kind(),
            "extension repository removed"
        );
        Ok(removed)
    }

    /// Validate all extension histories at one configured endpoint.
    pub fn verify(&self, name: &RepositoryName) -> Result<RepositoryVerification> {
        let repository = self.get(name)?;
        let index = repository.endpoint.local_index()?;
        let report = verify_local_index(&index)?;
        tracing::info!(
            event_name = "extension.repository.verify",
            repository = %repository.name,
            endpoint_kind = repository.endpoint.kind(),
            histories = report.history_count,
            releases = report.release_count,
            "extension repository verified"
        );
        Ok(report)
    }

    /// Search extension identity and display name across enabled repositories.
    pub fn search(&self, query: &ExtensionSearchQuery) -> Result<Vec<ExtensionSearchResult>> {
        let mut results = Vec::new();
        for repository in self.list()? {
            if repository.state == RepositoryState::Disabled {
                continue;
            }
            let index = repository.endpoint.local_index()?;
            for history in read_local_histories(&index)? {
                results.extend(
                    history
                        .releases()
                        .iter()
                        .filter(|release| query.matches(release))
                        .cloned()
                        .map(|release| ExtensionSearchResult {
                            repository: repository.clone(),
                            release,
                        }),
                );
            }
        }
        results.sort_by(|left, right| {
            left.repository
                .name
                .cmp(&right.repository.name)
                .then_with(|| {
                    left.release
                        .extension_id()
                        .cmp(right.release.extension_id())
                })
                .then_with(|| right.release.version().cmp(left.release.version()))
        });
        tracing::info!(
            event_name = "extension.catalog.search",
            query = %query.normalized,
            result_count = results.len(),
            "extension catalog searched"
        );
        Ok(results)
    }

    /// Resolve one exact extension artifact through an enabled repository.
    pub fn resolve(
        &self,
        repository_name: &RepositoryName,
        extension_id: &ExtensionId,
        selection: Selection,
        platform: &Platform,
    ) -> Result<ResolvedArtifact> {
        let repository = self.get(repository_name)?;
        if repository.state == RepositoryState::Disabled {
            return Err(DistributionError::RepositoryDisabled {
                name: repository.name,
            });
        }
        repository
            .endpoint
            .local_index()?
            .resolve(extension_id, selection, platform)
    }

    fn set_state(
        &self,
        name: &RepositoryName,
        state: RepositoryState,
    ) -> Result<ExtensionRepository> {
        let _guard = self.acquire()?;
        let mut repositories = self.load_unlocked()?;
        let repository = repositories
            .get_mut(name)
            .ok_or_else(|| DistributionError::RepositoryNotFound { name: name.clone() })?;
        repository.state = state;
        let updated = repository.clone();
        self.persist(&repositories)?;
        tracing::info!(
            event_name = "extension.repository.state_change",
            repository = %updated.name,
            endpoint_kind = updated.endpoint.kind(),
            state = updated.state.as_str(),
            "extension repository state changed"
        );
        Ok(updated)
    }

    fn acquire(&self) -> Result<StateGuard> {
        StateGuard::acquire(&self.home.extension_repositories_lock_file())
    }

    fn load_unlocked(&self) -> Result<BTreeMap<RepositoryName, ExtensionRepository>> {
        let path = self.home.extension_repositories_file();
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        let bytes = read_state_bytes(&path)?;
        let envelope: StateSchemaEnvelope = decode_state(&path, &bytes)?;
        if envelope.schema_version != REPOSITORIES_SCHEMA_VERSION {
            return Err(DistributionError::UnsupportedStateSchema {
                kind: "extension repositories",
                version: envelope.schema_version,
            });
        }
        let stored: RepositoryFile = decode_state(&path, &bytes)?;
        let mut repositories = BTreeMap::new();
        for repository in stored.repositories {
            let name = repository.name.clone();
            if repositories.insert(name.clone(), repository).is_some() {
                return Err(DistributionError::RepositoryAlreadyExists { name });
            }
        }
        Ok(repositories)
    }

    fn persist(&self, repositories: &BTreeMap<RepositoryName, ExtensionRepository>) -> Result<()> {
        atomic_write_json(
            &self.home.extension_repositories_file(),
            &RepositoryFile {
                schema_version: REPOSITORIES_SCHEMA_VERSION,
                repositories: repositories.values().cloned().collect(),
            },
        )
    }
}

fn verify_local_index(index: &LocalIndex) -> Result<RepositoryVerification> {
    let histories = read_local_histories(index)?;
    Ok(RepositoryVerification {
        history_count: histories.len(),
        release_count: histories
            .iter()
            .map(|history| history.releases().len())
            .sum(),
    })
}

fn read_local_histories(index: &LocalIndex) -> Result<Vec<ExtensionHistory>> {
    let extensions = index.root().join("extensions");
    let mut histories = fs::read_dir(&extensions)
        .map_err(|source| DistributionError::Io {
            path: extensions.clone(),
            source,
        })?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|source| DistributionError::Io {
                    path: extensions.clone(),
                    source,
                })
        })
        .collect::<Result<Vec<_>>>()?;
    histories.sort();

    let mut parsed = Vec::new();
    for path in histories {
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let canonical = fs::canonicalize(&path).map_err(|source| DistributionError::Io {
            path: path.clone(),
            source,
        })?;
        crate::local::ensure_contained(index.root(), &canonical)?;
        if !canonical.is_file() {
            return Err(DistributionError::Io {
                path: canonical,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "extension repository history is not a regular file",
                ),
            });
        }
        let expected = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                crate::error::invalid_value(
                    "extension id",
                    path.display().to_string(),
                    "expected a UTF-8 JSONL filename containing a portable extension id",
                )
            })
            .and_then(ExtensionId::parse)?;
        let bytes = fs::read(&canonical).map_err(|source| DistributionError::Io {
            path: canonical.clone(),
            source,
        })?;
        let history = ExtensionHistory::parse_jsonl(&bytes)?;
        if history.extension_id() != &expected {
            return Err(DistributionError::RepositoryHistoryIdentity {
                path: canonical,
                expected,
                actual: history.extension_id().clone(),
            });
        }
        parsed.push(history);
    }
    Ok(parsed)
}

fn normalize_search_text(value: &str) -> String {
    value
        .nfc()
        .collect::<String>()
        .chars()
        .case_fold()
        .nfc()
        .collect()
}
