//! Confined native filesystem adapter for portable workspace discovery.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use cap_std::{ambient_authority, fs::Dir};
use morphir_common::config::{MorphirConfig, env::env_config_value};
use morphir_workspace::{
    DiscoveryRequest, FileEntry, FileTree, RelativePath, WORKSPACE_DISCOVERY_PROTOCOL,
    WorkspaceSnapshot, discover_with_details,
};
use same_file::Handle;

use super::{
    discovery::{
        discover_config_candidates, native_global_config_candidates,
        native_system_config_candidates,
    },
    sources::{ConfigLoadOptions, EnvSelection, SourceSelection},
};

/// Discover a workspace by adapting a confined native directory to the
/// provider-neutral workspace protocol.
///
/// The granted `root` is canonicalized once and becomes the confinement
/// boundary. Native directories are traversed first, while confined directory
/// symlinks are recorded as aliases. Recognized entries are then copied from
/// the already-built real subtree to each alias without another filesystem
/// traversal. This keeps real paths visible, supports alias-only member globs,
/// and supports nested aliases. Alias edges already present in an expansion's
/// ancestry are skipped, so cycles cannot synthesize deeper paths indefinitely.
/// Fixed budgets bound alias edges, queued and processed expansions, generated
/// entries, and indexing/materialization work. Budget exhaustion returns the
/// stable `workspace.alias.resource-limit` code.
///
/// ```no_run
/// use morphir_devkit::{ConfigLoadOptions, discover_workspace};
/// use std::path::Path;
///
/// # fn main() -> anyhow::Result<()> {
/// let snapshot = discover_workspace(Path::new("."), &ConfigLoadOptions::default())?;
/// for project in snapshot.projects {
///     println!("{}: {}", project.relative_path.as_str(), project.name);
/// }
/// # Ok(())
/// # }
/// ```
pub fn discover_workspace(root: &Path, options: &ConfigLoadOptions) -> Result<WorkspaceSnapshot> {
    let request = build_workspace_discovery_request(root, options)?;
    morphir_workspace::discover(request)
        .into_result()
        .map_err(discovery_failure)
}

/// Native discovery output including decoded effective configurations from the
/// exact portable discovery pass and the canonical root bound by the adapter.
#[derive(Debug)]
pub struct NativeWorkspaceDiscovery {
    /// Canonical development root retained as the native confinement boundary.
    pub canonical_root: PathBuf,
    /// Provider-neutral discovery snapshot.
    pub snapshot: WorkspaceSnapshot,
    /// Fully merged root configuration.
    pub root_config: MorphirConfig,
    /// Fully merged configurations for valid projects, keyed by relative path.
    pub project_configs: BTreeMap<RelativePath, MorphirConfig>,
}

/// Discover a workspace and decode the exact effective configurations produced
/// by the portable engine without re-reading or re-merging any files.
pub fn discover_workspace_detailed(
    root: &Path,
    options: &ConfigLoadOptions,
) -> Result<NativeWorkspaceDiscovery> {
    let (canonical_root, request) = bind_workspace_discovery_request(root, options)?;
    let details = discover_with_details(request).map_err(discovery_failure)?;
    let root_config = decode_effective_config(details.root_effective)
        .context("Failed to decode effective root Morphir configuration")?;
    let project_configs = details
        .project_effective
        .into_iter()
        .map(|(path, value)| {
            if path == RelativePath::root() {
                Ok((path, root_config.clone()))
            } else {
                decode_effective_config(value)
                    .with_context(|| {
                        format!(
                            "Failed to decode effective Morphir configuration for `{}`",
                            path.as_str()
                        )
                    })
                    .map(|config| (path, config))
            }
        })
        .collect::<Result<_>>()?;
    Ok(NativeWorkspaceDiscovery {
        canonical_root,
        snapshot: details.snapshot,
        root_config,
        project_configs,
    })
}

fn decode_effective_config(mut value: serde_json::Value) -> Result<MorphirConfig> {
    if let Some(project) = value
        .get_mut("project")
        .and_then(serde_json::Value::as_object_mut)
    {
        project
            .entry("version")
            .or_insert_with(|| serde_json::Value::String(String::new()));
    }
    serde_json::from_value(value).map_err(Into::into)
}

/// Build the provider-neutral request used by [`discover_workspace`].
///
/// This lower-level API is useful to native hosts that need to serialize or
/// compare the exact request before running the pure discovery engine.
pub fn build_workspace_discovery_request(
    root: &Path,
    options: &ConfigLoadOptions,
) -> Result<DiscoveryRequest> {
    bind_workspace_discovery_request(root, options).map(|(_, request)| request)
}

fn bind_workspace_discovery_request(
    root: &Path,
    options: &ConfigLoadOptions,
) -> Result<(PathBuf, DiscoveryRequest)> {
    bind_workspace_discovery_request_with_hook(root, options, &mut |_| {})
}

fn bind_workspace_discovery_request_with_hook(
    root: &Path,
    options: &ConfigLoadOptions,
    root_opened_hook: &mut dyn FnMut(&Path),
) -> Result<(PathBuf, DiscoveryRequest)> {
    let granted_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .context("Failed to resolve relative development root")?
            .join(root)
    };
    let root_capability =
        Dir::open_ambient_dir(&granted_root, ambient_authority()).with_context(|| {
            format!(
                "Failed to open development root: {}",
                granted_root.display()
            )
        })?;
    root_opened_hook(&granted_root);
    let canonical_root = fs::canonicalize(&granted_root).map_err(|error| {
        anyhow!(
            "workspace.path.not-confined: development root changed after binding `{}`: {error}",
            granted_root.display()
        )
    })?;
    let bound_identity = Handle::from_file(
        root_capability
            .try_clone()
            .context("Failed to clone bound development root capability")?
            .into_std_file(),
    )
    .context("Failed to inspect bound development root identity")?;
    let canonical_identity = Handle::from_path(&canonical_root).map_err(|error| {
        anyhow!(
            "workspace.path.not-confined: development root changed after binding `{}` while verifying `{}`: {error}",
            granted_root.display(),
            canonical_root.display()
        )
    })?;
    if bound_identity != canonical_identity {
        bail!(
            "workspace.path.not-confined: development root changed after binding `{}`; canonical path now identifies `{}`",
            granted_root.display(),
            canonical_root.display()
        );
    }
    let mut development_root =
        build_tree_from_capability(&root_capability, &canonical_root, &granted_root)?;
    apply_user_override_selection(&mut development_root, &options.user_override)?;

    let request = DiscoveryRequest {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        development_root,
        morphir_home: selected_mount(
            &options.global,
            native_global_config_candidates,
            "global user",
        )?,
        system_config: selected_mount(
            &options.system,
            || native_system_config_candidates().to_vec(),
            "system",
        )?,
        environment: selected_environment(options),
        cli_overlay: serde_json::json!({}),
    };
    Ok((canonical_root, request))
}

fn discovery_failure(failure: morphir_workspace::DiscoveryFailure) -> anyhow::Error {
    let path = failure
        .path
        .as_ref()
        .map(|path| format!(" at `{}`", path.as_str()))
        .unwrap_or_default();
    anyhow!("{}: {}{path}", failure.code, failure.message)
}

fn build_tree_from_capability(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
) -> Result<FileTree> {
    build_tree_with(
        root,
        canonical_root,
        granted_root,
        AliasBudgets::DEFAULT,
        &mut |_, _| {},
    )
}

fn build_tree_with(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
    budgets: AliasBudgets,
    boundary_hook: &mut dyn FnMut(BoundaryEvent, &RelativePath),
) -> Result<FileTree> {
    let mut entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory)]);
    let mut visited_directories = BTreeSet::from([RelativePath::root()]);
    let mut directory_aliases = Vec::new();
    Traversal {
        root,
        canonical_root,
        granted_root,
        visited_directories: &mut visited_directories,
        directory_aliases: &mut directory_aliases,
        entries: &mut entries,
        budgets,
        boundary_hook,
    }
    .walk(root, &RelativePath::root(), &RelativePath::root())?;
    materialize_directory_aliases(&mut directory_aliases, &mut entries, budgets)?;
    Ok(FileTree { entries })
}

#[derive(Debug)]
struct DirectoryAlias {
    lexical_path: RelativePath,
    canonical_target: RelativePath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundaryEvent {
    BeforeOpenDirectory,
    BeforeReadConfig,
}

struct Traversal<'a> {
    root: &'a Dir,
    canonical_root: &'a Path,
    granted_root: &'a Path,
    visited_directories: &'a mut BTreeSet<RelativePath>,
    directory_aliases: &'a mut Vec<DirectoryAlias>,
    entries: &'a mut BTreeMap<RelativePath, FileEntry>,
    budgets: AliasBudgets,
    boundary_hook: &'a mut dyn FnMut(BoundaryEvent, &RelativePath),
}

impl Traversal<'_> {
    fn walk(
        &mut self,
        directory: &Dir,
        canonical_directory: &RelativePath,
        lexical_directory: &RelativePath,
    ) -> Result<()> {
        let mut children = directory
            .entries()
            .with_context(|| format!("Failed to read directory `{}`", lexical_directory.as_str()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.file_name());

        for child in children {
            let file_name = child.file_name();
            let canonical_child = join_native_path(canonical_directory, &file_name);
            let link_metadata = self
                .root
                .symlink_metadata(&canonical_child)
                .with_context(|| format!("Failed to inspect `{}`", canonical_child.display()))?;
            let Some(file_name) = file_name.to_str() else {
                if link_metadata.file_type().is_symlink() {
                    confined_canonicalize(
                        self.root,
                        self.canonical_root,
                        self.granted_root,
                        &canonical_child,
                        None,
                    )?;
                }
                continue;
            };
            let lexical_path = lexical_directory.join(file_name).map_err(|error| {
                anyhow!(
                    "workspace.path.not-confined: invalid path below `{}`: {error}",
                    lexical_directory.as_str()
                )
            })?;
            if link_metadata.file_type().is_symlink() {
                let canonical_target = confined_canonicalize(
                    self.root,
                    self.canonical_root,
                    self.granted_root,
                    &canonical_child,
                    Some(&lexical_path),
                )?;
                let target_metadata = self
                    .root
                    .metadata(relative_native_path(&canonical_target))
                    .with_context(|| {
                        format!(
                            "Failed to inspect symlink target `{}` for `{}`",
                            canonical_target.as_str(),
                            lexical_path.as_str()
                        )
                    })?;
                if target_metadata.is_dir() {
                    record_directory_alias(self.directory_aliases, self.budgets, || {
                        DirectoryAlias {
                            lexical_path,
                            canonical_target,
                        }
                    })?;
                } else if target_metadata.is_file() && is_recognized_config(&lexical_path) {
                    let confined_target = relative_native_path(&canonical_target).to_path_buf();
                    insert_config_file(
                        self.root,
                        self.entries,
                        lexical_path,
                        confined_target,
                        self.boundary_hook,
                    )?;
                }
                continue;
            }

            if link_metadata.is_dir() {
                let canonical_directory = confined_canonicalize(
                    self.root,
                    self.canonical_root,
                    self.granted_root,
                    &canonical_child,
                    Some(&lexical_path),
                )?;
                self.entries
                    .insert(lexical_path.clone(), FileEntry::Directory);
                if self.visited_directories.insert(canonical_directory.clone()) {
                    (self.boundary_hook)(BoundaryEvent::BeforeOpenDirectory, &lexical_path);
                    let opened = self
                        .root
                        .open_dir(relative_native_path(&canonical_directory))
                        .with_context(|| {
                            format!(
                                "Failed to open confined directory `{}`",
                                lexical_path.as_str()
                            )
                        })?;
                    self.walk(&opened, &canonical_directory, &lexical_path)?;
                }
            } else if link_metadata.is_file() && is_recognized_config(&lexical_path) {
                insert_config_file(
                    self.root,
                    self.entries,
                    lexical_path,
                    canonical_child,
                    self.boundary_hook,
                )?;
            }
        }
        Ok(())
    }
}

fn record_directory_alias(
    aliases: &mut Vec<DirectoryAlias>,
    budgets: AliasBudgets,
    make_alias: impl FnOnce() -> DirectoryAlias,
) -> Result<()> {
    if aliases.len() >= budgets.alias_edges {
        return alias_resource_limit("alias edges", budgets.alias_edges);
    }
    aliases.push(make_alias());
    Ok(())
}

fn materialize_directory_aliases(
    aliases: &mut [DirectoryAlias],
    entries: &mut BTreeMap<RelativePath, FileEntry>,
    budgets: AliasBudgets,
) -> Result<()> {
    aliases.sort_by(|left, right| left.lexical_path.cmp(&right.lexical_path));
    if aliases.len() > budgets.alias_edges {
        return alias_resource_limit("alias edges", budgets.alias_edges);
    }
    if aliases.is_empty() {
        return Ok(());
    }
    let mut stats = AliasStats::default();
    let mut real_entries = BTreeMap::new();
    for (path, entry) in entries.iter() {
        stats.work(budgets)?;
        real_entries.insert(path.clone(), entry.clone());
    }
    let mut edges = Vec::with_capacity(aliases.len());
    for alias in aliases {
        stats.work(budgets)?;
        edges.push(ResolvedAlias {
            lexical_path: alias.lexical_path.clone(),
            real_target: alias.canonical_target.clone(),
        });
    }
    let mut edge_index = BTreeMap::new();
    for (edge_id, edge) in edges.iter().enumerate() {
        stats.work(budgets)?;
        edge_index.insert(edge.lexical_path.clone(), edge_id);
    }
    let mut worklist = BTreeSet::new();
    for (edge_id, edge) in edges.iter().enumerate() {
        stats.enqueue(budgets)?;
        worklist.insert(AliasExpansion {
            lexical_path: edge.lexical_path.clone(),
            edge_id,
            ancestry: vec![edge_id],
        });
    }
    let mut indexes = BTreeMap::<RelativePath, AliasIndex>::new();

    while let Some(expansion) = worklist.pop_first() {
        stats.process(budgets)?;
        let edge = &edges[expansion.edge_id];
        if !indexes.contains_key(&edge.real_target) {
            let index = build_alias_index(
                &edge.real_target,
                &real_entries,
                &edge_index,
                budgets,
                &mut stats,
            )?;
            stats.work(budgets)?;
            indexes.insert(edge.real_target.clone(), index);
        }
        let index = indexes
            .get(&edge.real_target)
            .expect("alias target index was just built");
        for (suffix, entry) in &index.entries {
            stats.work(budgets)?;
            let alias_path =
                if suffix.is_empty() {
                    expansion.lexical_path.clone()
                } else {
                    expansion.lexical_path.join(suffix.as_str()).map_err(|error| {
                    anyhow!(
                        "workspace.path.not-confined: cannot materialize alias `{}`: {error}",
                        expansion.lexical_path.as_str()
                    )
                })?
                };
            if let std::collections::btree_map::Entry::Vacant(slot) = entries.entry(alias_path) {
                stats.generate(budgets)?;
                slot.insert(entry.clone());
            }
        }

        for (suffix, nested_id) in &index.nested_aliases {
            if expansion.ancestry.binary_search(nested_id).is_ok() {
                continue;
            }
            stats.work(budgets)?;
            let nested_lexical_path = if suffix.is_empty() {
                expansion.lexical_path.clone()
            } else {
                expansion.lexical_path.join(suffix.as_str()).map_err(|error| {
                    anyhow!(
                        "workspace.path.not-confined: cannot materialize nested alias below `{}`: {error}",
                        expansion.lexical_path.as_str()
                    )
                })?
            };
            let mut ancestry = expansion.ancestry.clone();
            ancestry.push(*nested_id);
            ancestry.sort_unstable();
            let nested = AliasExpansion {
                lexical_path: nested_lexical_path,
                edge_id: *nested_id,
                ancestry,
            };
            if !worklist.contains(&nested) {
                stats.enqueue(budgets)?;
                worklist.insert(nested);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct AliasBudgets {
    alias_edges: usize,
    queued_expansions: usize,
    processed_expansions: usize,
    generated_entries: usize,
    total_work: usize,
}

impl AliasBudgets {
    const DEFAULT: Self = Self {
        alias_edges: 4_096,
        queued_expansions: 32_768,
        processed_expansions: 32_768,
        generated_entries: 262_144,
        total_work: 2_000_000,
    };
}

#[derive(Default)]
struct AliasStats {
    queued_expansions: usize,
    processed_expansions: usize,
    generated_entries: usize,
    total_work: usize,
}

impl AliasStats {
    fn enqueue(&mut self, budgets: AliasBudgets) -> Result<()> {
        increment_bounded(
            &mut self.queued_expansions,
            budgets.queued_expansions,
            "queued expansions",
        )?;
        self.work(budgets)
    }

    fn process(&mut self, budgets: AliasBudgets) -> Result<()> {
        increment_bounded(
            &mut self.processed_expansions,
            budgets.processed_expansions,
            "processed expansions",
        )?;
        self.work(budgets)
    }

    fn generate(&mut self, budgets: AliasBudgets) -> Result<()> {
        increment_bounded(
            &mut self.generated_entries,
            budgets.generated_entries,
            "generated entries",
        )?;
        self.work(budgets)
    }

    fn work(&mut self, budgets: AliasBudgets) -> Result<()> {
        increment_bounded(&mut self.total_work, budgets.total_work, "total work")
    }
}

fn increment_bounded(value: &mut usize, limit: usize, resource: &str) -> Result<()> {
    if *value >= limit {
        return alias_resource_limit(resource, limit);
    }
    *value += 1;
    Ok(())
}

fn alias_resource_limit<T>(resource: &str, limit: usize) -> Result<T> {
    bail!(
        "workspace.alias.resource-limit: confined alias graph exceeded fixed {resource} budget {limit}"
    )
}

struct AliasIndex {
    entries: Vec<(String, FileEntry)>,
    nested_aliases: Vec<(String, usize)>,
}

fn build_alias_index(
    target: &RelativePath,
    real_entries: &BTreeMap<RelativePath, FileEntry>,
    edge_index: &BTreeMap<RelativePath, usize>,
    budgets: AliasBudgets,
    stats: &mut AliasStats,
) -> Result<AliasIndex> {
    let entries = index_real_subtree(target, real_entries, budgets, stats)?;
    let nested_aliases = index_alias_subtree(target, edge_index, budgets, stats)?;
    Ok(AliasIndex {
        entries,
        nested_aliases,
    })
}

fn index_alias_subtree(
    target: &RelativePath,
    edge_index: &BTreeMap<RelativePath, usize>,
    budgets: AliasBudgets,
    stats: &mut AliasStats,
) -> Result<Vec<(String, usize)>> {
    if target == &RelativePath::root() {
        return edge_index
            .iter()
            .map(|(path, edge_id)| {
                stats.work(budgets)?;
                Ok((
                    relative_suffix(path, target)
                        .expect("every confined alias path is below the root")
                        .to_owned(),
                    *edge_id,
                ))
            })
            .collect();
    }

    let mut indexed = Vec::new();
    stats.work(budgets)?;
    if let Some(edge_id) = edge_index.get(target) {
        indexed.push((String::new(), *edge_id));
    }
    stats.work(budgets)?;
    let prefix = format!("{}/", target.as_str());
    let upper = RelativePath::parse(format!("{}0", target.as_str()))
        .expect("appending a confined component character remains confined");
    for (path, edge_id) in edge_index.range((
        std::ops::Bound::Excluded(target.clone()),
        std::ops::Bound::Excluded(upper),
    )) {
        stats.work(budgets)?;
        if !path.as_str().starts_with(&prefix) {
            continue;
        }
        indexed.push((path.as_str()[prefix.len()..].to_owned(), *edge_id));
    }
    Ok(indexed)
}

fn index_real_subtree(
    target: &RelativePath,
    real_entries: &BTreeMap<RelativePath, FileEntry>,
    budgets: AliasBudgets,
    stats: &mut AliasStats,
) -> Result<Vec<(String, FileEntry)>> {
    if target == &RelativePath::root() {
        return real_entries
            .iter()
            .map(|(path, entry)| {
                stats.work(budgets)?;
                Ok((
                    relative_suffix(path, target)
                        .expect("every confined path is below the root")
                        .to_owned(),
                    entry.clone(),
                ))
            })
            .collect();
    }

    let mut indexed = Vec::new();
    stats.work(budgets)?;
    if let Some(entry) = real_entries.get(target) {
        stats.work(budgets)?;
        indexed.push((String::new(), entry.clone()));
    }
    stats.work(budgets)?;
    let prefix = format!("{}/", target.as_str());
    let upper = RelativePath::parse(format!("{}0", target.as_str()))
        .expect("appending a confined component character remains confined");
    for (path, entry) in real_entries.range((
        std::ops::Bound::Excluded(target.clone()),
        std::ops::Bound::Excluded(upper),
    )) {
        stats.work(budgets)?;
        if !path.as_str().starts_with(&prefix) {
            continue;
        }
        stats.work(budgets)?;
        indexed.push((path.as_str()[prefix.len()..].to_owned(), entry.clone()));
    }
    Ok(indexed)
}

struct ResolvedAlias {
    lexical_path: RelativePath,
    real_target: RelativePath,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AliasExpansion {
    lexical_path: RelativePath,
    edge_id: usize,
    ancestry: Vec<usize>,
}

fn native_path_to_relative(path: &Path) -> Result<RelativePath> {
    if path.as_os_str().is_empty() || path == Path::new(".") {
        return Ok(RelativePath::root());
    }
    let normalized = path
        .components()
        .map(|component| {
            component.as_os_str().to_str().ok_or_else(|| {
                anyhow!(
                    "workspace.path.not-confined: confined path `{}` is not UTF-8",
                    path.display()
                )
            })
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    RelativePath::parse(normalized).map_err(|error| anyhow!(error))
}

fn relative_suffix<'a>(path: &'a RelativePath, root: &RelativePath) -> Option<&'a str> {
    if root.as_str() == "." {
        return Some(if path.as_str() == "." {
            ""
        } else {
            path.as_str()
        });
    }
    if path == root {
        return Some("");
    }
    path.as_str().strip_prefix(root.as_str())?.strip_prefix('/')
}

fn confined_canonicalize(
    root: &Dir,
    canonical_root: &Path,
    granted_root: &Path,
    path: &Path,
    lexical_path: Option<&RelativePath>,
) -> Result<RelativePath> {
    match root.canonicalize(path) {
        Ok(path) => native_path_to_relative(&path),
        Err(first_error) => {
            let target = root.read_link_contents(path).ok();
            if let Some(target) = target.as_deref().filter(|target| target.is_absolute()) {
                for accepted_root in [canonical_root, granted_root] {
                    if let Ok(relative) = target.strip_prefix(accepted_root)
                        && let Ok(path) = root.canonicalize(relative)
                    {
                        return native_path_to_relative(&path);
                    }
                }
            }
            let link = lexical_path
                .map(RelativePath::as_str)
                .unwrap_or_else(|| path.to_str().unwrap_or("<non-UTF-8>"));
            let target = target
                .map(|target| target.display().to_string())
                .unwrap_or_else(|| "<unresolved>".to_owned());
            bail!(
                "workspace.path.not-confined: `{link}` resolves through `{target}` outside the bound development root or could not be resolved safely: {first_error}"
            )
        }
    }
}

fn relative_native_path(path: &RelativePath) -> &Path {
    Path::new(path.as_str())
}

fn join_native_path(directory: &RelativePath, name: &OsStr) -> PathBuf {
    if directory.as_str() == "." {
        PathBuf::from(name)
    } else {
        Path::new(directory.as_str()).join(name)
    }
}

fn insert_config_file(
    root: &Dir,
    entries: &mut BTreeMap<RelativePath, FileEntry>,
    lexical_path: RelativePath,
    confined_path: PathBuf,
    boundary_hook: &mut dyn FnMut(BoundaryEvent, &RelativePath),
) -> Result<()> {
    boundary_hook(BoundaryEvent::BeforeReadConfig, &lexical_path);
    let text = root.read_to_string(&confined_path).with_context(|| {
        format!(
            "Failed to read confined UTF-8 Morphir configuration `{}` from `{}`",
            lexical_path.as_str(),
            confined_path.display()
        )
    })?;
    entries.insert(lexical_path, FileEntry::File { text });
    Ok(())
}

fn is_recognized_config(path: &RelativePath) -> bool {
    let components = path.as_str().split('/').collect::<Vec<_>>();
    let Some(name) = components.last().copied() else {
        return false;
    };
    match name {
        "morphir.toml" | "morphir.yaml" | "morphir.json" | "morphir.user.toml"
        | "morphir.user.yaml" => true,
        "config.toml" | "config.yaml" | "config.user.toml" | "config.user.yaml" => {
            components.len() >= 3
                && components[components.len() - 2] == "morphir"
                && components[components.len() - 3] == ".config"
        }
        _ => false,
    }
}

fn selected_mount(
    selection: &SourceSelection,
    candidates: impl FnOnce() -> Vec<PathBuf>,
    description: &str,
) -> Result<Option<FileTree>> {
    let selected = match selection {
        SourceSelection::Skip => return Ok(None),
        SourceSelection::Explicit(path) => match fs::symlink_metadata(path) {
            Ok(_) => Some(path.clone()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to inspect explicit {description} config: {}",
                        path.display()
                    )
                });
            }
        },
        SourceSelection::Discover => discover_config_candidates(&candidates())?,
    };
    selected
        .map(|path| config_mount(&path, description))
        .transpose()
}

fn config_mount(path: &Path, description: &str) -> Result<FileTree> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Failed to inspect {description} config: {}", path.display()))?;
    let read_path = if metadata.file_type().is_symlink() {
        fs::canonicalize(path).with_context(|| {
            format!("Failed to resolve {description} config: {}", path.display())
        })?
    } else {
        path.to_path_buf()
    };
    let virtual_name = virtual_primary_name(path).ok_or_else(|| {
        anyhow!(
            "Unsupported {description} config serialization at {}; expected TOML, YAML, or JSON",
            path.display()
        )
    })?;
    let text = fs::read_to_string(&read_path)
        .with_context(|| format!("Failed to read {description} config: {}", path.display()))?;
    Ok(FileTree {
        entries: BTreeMap::from([
            (RelativePath::root(), FileEntry::Directory),
            (
                RelativePath::parse(virtual_name).expect("virtual primary name is confined"),
                FileEntry::File { text },
            ),
        ]),
    })
}

fn virtual_primary_name(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(OsStr::to_str) {
        Some("toml") => Some("morphir.toml"),
        Some("yaml") => Some("morphir.yaml"),
        Some("json") => Some("morphir.json"),
        _ => None,
    }
}

fn apply_user_override_selection(tree: &mut FileTree, selection: &SourceSelection) -> Result<()> {
    match selection {
        SourceSelection::Discover => Ok(()),
        SourceSelection::Skip => {
            tree.entries.retain(|path, entry| {
                !is_user_override_path(path) || !matches!(entry, FileEntry::File { .. })
            });
            Ok(())
        }
        SourceSelection::Explicit(source) => {
            let primary = modern_root_primary(tree, source)?;
            let candidates = morphir_workspace::config::adjacent_user_candidates(&primary);
            for candidate in &candidates {
                if matches!(tree.entries.get(candidate), Some(FileEntry::File { .. })) {
                    tree.entries.remove(candidate);
                }
            }
            materialize_explicit_user_override(tree, source, &primary)
        }
    }
}

fn is_user_override_path(path: &RelativePath) -> bool {
    matches!(
        path.as_str().rsplit('/').next(),
        Some("morphir.user.toml" | "morphir.user.yaml" | "config.user.toml" | "config.user.yaml")
    )
}

fn modern_root_primary(tree: &FileTree, source: &Path) -> Result<RelativePath> {
    let root = RelativePath::root();
    let primaries = morphir_workspace::config::found_primary_candidates(tree, &root);
    match primaries.as_slice() {
        [primary] if primary.as_str() != "morphir.json" => Ok(primary.clone()),
        [..] => {
            bail!(
                "Cannot apply explicit user override {}; explicit user overrides require exactly one modern TOML/YAML root config",
                source.display()
            )
        }
    }
}

fn materialize_explicit_user_override(
    tree: &mut FileTree,
    source: &Path,
    primary: &RelativePath,
) -> Result<()> {
    let source_name = source.display();
    let serialization = match source.extension().and_then(OsStr::to_str) {
        Some("toml") => "toml",
        Some("yaml") => "yaml",
        _ => {
            bail!(
                "Explicit user override {source_name} is unsupported; explicit user overrides require a modern TOML/YAML root config and TOML/YAML override"
            )
        }
    };
    let text = fs::read_to_string(source)
        .with_context(|| format!("Failed to read explicit user override: {source_name}"))?;
    morphir_config::parse_config(&source.to_string_lossy(), &text)
        .with_context(|| format!("Invalid explicit user override: {source_name}"))?;

    let candidates = morphir_workspace::config::adjacent_user_candidates(primary);
    let target = candidates
        .into_iter()
        .find(|candidate| candidate.as_str().ends_with(serialization))
        .ok_or_else(|| {
            anyhow!(
                "Cannot apply explicit user override {source_name}; explicit user overrides require a modern TOML/YAML root config"
            )
        })?;
    if tree.entries.contains_key(&target) {
        bail!(
            "Cannot materialize explicit user override {} at `{}` because that path is already occupied",
            source.display(),
            target.as_str()
        );
    }
    tree.entries.insert(target, FileEntry::File { text });
    Ok(())
}

fn selected_environment(options: &ConfigLoadOptions) -> BTreeMap<String, String> {
    let vars = match &options.env {
        EnvSelection::Skip => return BTreeMap::new(),
        EnvSelection::Explicit(vars) => vars.clone(),
        EnvSelection::Process => std::env::vars_os()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect(),
    };
    let configured_prefix = options
        .env_prefix
        .trim_end_matches('_')
        .to_ascii_uppercase();
    let prefix = format!("{configured_prefix}_");

    vars.into_iter()
        .filter(|(key, value)| {
            key != "MORPHIR_HOME"
                && key != "MORPHIR_LOG_DIR"
                && env_config_value(&options.env_prefix, [(key, value)])
                    .as_object()
                    .is_some_and(|object| !object.is_empty())
        })
        .filter_map(|(key, value)| {
            let uppercase = key.to_ascii_uppercase();
            uppercase.strip_prefix(&prefix).map(|suffix| {
                let suffix = suffix.trim_start_matches('_');
                let portable_key = if configured_prefix == "MORPHIR" {
                    format!("MORPHIR_{suffix}")
                } else {
                    format!("MORPHIR__{suffix}")
                };
                (portable_key, value)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[cfg(unix)]
    fn open_capability(path: &Path) -> Dir {
        Dir::open_ambient_dir(path, ambient_authority()).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn root_replaced_after_capability_open_is_rejected_by_identity() {
        let parent = tempfile::tempdir().unwrap();
        let grant = parent.path().join("granted-root");
        fs::create_dir(&grant).unwrap();
        fs::write(
            grant.join("morphir.toml"),
            "[project]\nname = 'inside/project'\n",
        )
        .unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(
            outside.path().join("morphir.toml"),
            "[project]\nname = 'external/project'\n",
        )
        .unwrap();
        let held = parent.path().join("held-original");

        let error = bind_workspace_discovery_request_with_hook(
            &grant,
            &ConfigLoadOptions::project_only(),
            &mut |_| {
                fs::rename(&grant, &held).unwrap();
                symlink(outside.path(), &grant).unwrap();
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("workspace.path.not-confined"));
        assert!(error.to_string().contains("development root changed"));
        assert!(error.to_string().contains(&grant.display().to_string()));
        assert!(!error.to_string().contains("external/project"));
    }

    #[cfg(unix)]
    #[test]
    fn config_replaced_by_external_symlink_before_read_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let config = root.path().join("morphir.toml");
        fs::write(&config, "[project]\nname = 'inside/project'\n").unwrap();
        let external = outside.path().join("external.toml");
        fs::write(&external, "[project]\nname = 'external/project'\n").unwrap();
        let cap = open_capability(root.path());
        let mut replaced = false;

        let error = build_tree_with(
            &cap,
            root.path(),
            root.path(),
            AliasBudgets::DEFAULT,
            &mut |event, path| {
                if !replaced
                    && event == BoundaryEvent::BeforeReadConfig
                    && path.as_str() == "morphir.toml"
                {
                    fs::rename(&config, root.path().join("original.toml")).unwrap();
                    symlink(&external, &config).unwrap();
                    replaced = true;
                }
            },
        )
        .unwrap_err();

        assert!(replaced);
        assert!(error.to_string().contains("Failed to read confined"));
        assert!(!error.to_string().contains("external/project"));
    }

    #[cfg(unix)]
    #[test]
    fn absolute_internal_config_symlink_is_read_through_the_capability() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("actual-config.toml");
        fs::write(&target, "[project]\nname = 'inside/project'\n").unwrap();
        symlink(&target, root.path().join("morphir.toml")).unwrap();
        let cap = open_capability(root.path());

        let tree = build_tree_with(
            &cap,
            root.path(),
            root.path(),
            AliasBudgets::DEFAULT,
            &mut |_, _| {},
        )
        .unwrap();

        assert_eq!(
            tree.file_text(&RelativePath::parse("morphir.toml").unwrap()),
            Some("[project]\nname = 'inside/project'\n")
        );
        assert!(
            !tree
                .entries
                .contains_key(&RelativePath::parse("actual-config.toml").unwrap())
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_replaced_by_external_symlink_before_open_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = ['packages/*']\n",
        )
        .unwrap();
        let packages = root.path().join("packages");
        fs::create_dir(&packages).unwrap();
        let external_packages = outside.path().join("packages");
        fs::create_dir(&external_packages).unwrap();
        fs::write(
            external_packages.join("morphir.toml"),
            "[project]\nname = 'external/project'\n",
        )
        .unwrap();
        let cap = open_capability(root.path());
        let mut replaced = false;

        let error = build_tree_with(
            &cap,
            root.path(),
            root.path(),
            AliasBudgets::DEFAULT,
            &mut |event, path| {
                if !replaced
                    && event == BoundaryEvent::BeforeOpenDirectory
                    && path.as_str() == "packages"
                {
                    fs::rename(&packages, root.path().join("original-packages")).unwrap();
                    symlink(&external_packages, &packages).unwrap();
                    replaced = true;
                }
            },
        )
        .unwrap_err();

        assert!(replaced);
        assert!(
            error
                .to_string()
                .contains("Failed to open confined directory")
        );
        assert!(!error.to_string().contains("external/project"));
    }

    #[cfg(unix)]
    #[test]
    fn alias_budget_is_fixed_and_reports_a_stable_code() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = ['alias']\n",
        )
        .unwrap();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("morphir.toml"), "[project]\nname = 'acme/real'\n").unwrap();
        symlink(&real, root.path().join("alias")).unwrap();
        let cap = open_capability(root.path());
        let budgets = AliasBudgets {
            alias_edges: 0,
            ..AliasBudgets::DEFAULT
        };

        let error =
            build_tree_with(&cap, root.path(), root.path(), budgets, &mut |_, _| {}).unwrap_err();

        assert!(error.to_string().contains("workspace.alias.resource-limit"));
        assert!(error.to_string().contains("alias edges budget 0"));
    }

    #[test]
    fn traversal_rejects_the_first_alias_edge_over_budget_before_storing_it() {
        let mut aliases = Vec::new();
        let mut allocated = 0;
        let budgets = AliasBudgets {
            alias_edges: 1,
            ..AliasBudgets::DEFAULT
        };
        record_directory_alias(&mut aliases, budgets, || {
            allocated += 1;
            DirectoryAlias {
                lexical_path: RelativePath::parse("alias-a").unwrap(),
                canonical_target: RelativePath::parse("real-a").unwrap(),
            }
        })
        .unwrap();

        let error = record_directory_alias(&mut aliases, budgets, || {
            allocated += 1;
            DirectoryAlias {
                lexical_path: RelativePath::parse("alias-b").unwrap(),
                canonical_target: RelativePath::parse("real-b").unwrap(),
            }
        })
        .unwrap_err();

        assert_eq!(aliases.len(), 1);
        assert_eq!(allocated, 1);
        assert!(error.to_string().contains("workspace.alias.resource-limit"));
        assert!(error.to_string().contains("alias edges budget 1"));
    }

    #[cfg(unix)]
    #[test]
    fn alias_work_budget_is_checked_before_snapshot_cloning() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("morphir.toml"),
            "[workspace]\nmembers = ['alias']\n",
        )
        .unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        symlink(root.path().join("real"), root.path().join("alias")).unwrap();
        let cap = open_capability(root.path());
        let budgets = AliasBudgets {
            total_work: 0,
            ..AliasBudgets::DEFAULT
        };

        let error =
            build_tree_with(&cap, root.path(), root.path(), budgets, &mut |_, _| {}).unwrap_err();

        assert!(error.to_string().contains("workspace.alias.resource-limit"));
        assert!(error.to_string().contains("total work budget 0"));
    }

    #[test]
    fn many_distinct_shallow_alias_targets_use_bounded_subtree_index_work() {
        let mut entries = BTreeMap::from([
            (RelativePath::root(), FileEntry::Directory),
            (RelativePath::parse("real").unwrap(), FileEntry::Directory),
        ]);
        let mut aliases = Vec::new();
        for index in 0..128 {
            let target = RelativePath::parse(format!("real/{index:03}")).unwrap();
            entries.insert(target.clone(), FileEntry::Directory);
            entries.insert(
                target.join("morphir.toml").unwrap(),
                FileEntry::File {
                    text: format!("[project]\nname = 'acme/project-{index:03}'\n"),
                },
            );
            aliases.push(DirectoryAlias {
                lexical_path: RelativePath::parse(format!("alias/{index:03}")).unwrap(),
                canonical_target: target,
            });
        }
        let budgets = AliasBudgets {
            total_work: 10_000,
            ..AliasBudgets::DEFAULT
        };

        materialize_directory_aliases(&mut aliases, &mut entries, budgets).unwrap();

        assert_eq!(
            entries
                .keys()
                .filter(|path| path.as_str().starts_with("alias/")
                    && path.as_str().ends_with("morphir.toml"))
                .count(),
            128
        );
    }

    #[test]
    fn punctuation_sibling_does_not_hide_direct_alias_descendants() {
        let target = RelativePath::parse("real/pkg").unwrap();
        let punctuation_sibling = RelativePath::parse("real/pkg!shadow").unwrap();
        let config = target.join("morphir.toml").unwrap();
        assert!(target < punctuation_sibling);
        assert!(punctuation_sibling < config);
        let mut entries = BTreeMap::from([
            (RelativePath::root(), FileEntry::Directory),
            (target.clone(), FileEntry::Directory),
            (punctuation_sibling, FileEntry::Directory),
            (
                config,
                FileEntry::File {
                    text: "[project]\nname = 'acme/pkg'\n".to_owned(),
                },
            ),
        ]);
        let mut aliases = [DirectoryAlias {
            lexical_path: RelativePath::parse("alias/pkg").unwrap(),
            canonical_target: target,
        }];

        materialize_directory_aliases(&mut aliases, &mut entries, AliasBudgets::DEFAULT).unwrap();

        assert!(entries.contains_key(&RelativePath::parse("alias/pkg/morphir.toml").unwrap()));
    }

    #[test]
    fn punctuation_sibling_does_not_hide_nested_alias_edges() {
        let target = RelativePath::parse("real/pkg").unwrap();
        let punctuation_alias = RelativePath::parse("real/pkg!shadow").unwrap();
        let nested_alias = RelativePath::parse("real/pkg/linked").unwrap();
        assert!(target < punctuation_alias);
        assert!(punctuation_alias < nested_alias);
        let orders = RelativePath::parse("projects/orders").unwrap();
        let shadow = RelativePath::parse("projects/shadow").unwrap();
        let mut entries = BTreeMap::from([
            (RelativePath::root(), FileEntry::Directory),
            (target.clone(), FileEntry::Directory),
            (orders.clone(), FileEntry::Directory),
            (shadow.clone(), FileEntry::Directory),
            (
                orders.join("morphir.toml").unwrap(),
                FileEntry::File {
                    text: "[project]\nname = 'acme/orders'\n".to_owned(),
                },
            ),
        ]);
        let mut aliases = [
            DirectoryAlias {
                lexical_path: RelativePath::parse("alias/pkg").unwrap(),
                canonical_target: target,
            },
            DirectoryAlias {
                lexical_path: punctuation_alias,
                canonical_target: shadow,
            },
            DirectoryAlias {
                lexical_path: nested_alias,
                canonical_target: orders,
            },
        ];

        materialize_directory_aliases(&mut aliases, &mut entries, AliasBudgets::DEFAULT).unwrap();

        assert!(
            entries.contains_key(&RelativePath::parse("alias/pkg/linked/morphir.toml").unwrap())
        );
    }
}
