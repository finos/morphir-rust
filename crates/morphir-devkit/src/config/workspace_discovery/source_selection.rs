//! Native capture of an explicit source selection for ad-hoc discovery.
//!
//! Configuration discovery collects manifests; this collects *sources*. It
//! settles the host-tier rules for a selection before any provider is asked
//! about it — one root, canonical and deduplicated paths, confinement, text
//! only, a byte budget — and keeps the text it read, so a compile submits the
//! exact bytes discovery saw instead of reading every file twice.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use morphir_workspace::{
    DiscoveryPurpose, DiscoveryRequest, FileEntry, FileTree, ProjectSource, RelativePath,
    SourceSelection,
};

/// The default request-wide limit on captured source bytes, matching the
/// configuration traversal's payload budget.
pub const DEFAULT_SOURCE_BYTES: usize = 64 * 1024 * 1024;

/// What to capture for an ad-hoc discovery request.
#[derive(Clone, Debug)]
pub struct SourceSelectionOptions<'a> {
    /// The selected files or directories. Relative paths resolve against the
    /// current directory.
    pub inputs: &'a [PathBuf],
    /// The suffixes a directory input collects, as a provider declares them
    /// (for example `.elm`). A file named explicitly is taken whatever its
    /// suffix: the caller chose it.
    pub file_extensions: &'a [String],
    /// The one language every selected source is in.
    pub language_id: &'a str,
    /// The manifest whose identity the selection borrows, when it has one.
    /// `None` synthesizes a project from the sources alone.
    pub manifest: Option<&'a Path>,
    /// The request-wide limit on captured source bytes.
    pub byte_limit: usize,
}

/// One captured source, as discovery saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedSource {
    /// The canonical native path.
    pub path: PathBuf,
    /// The path relative to the request's development root.
    pub relative: RelativePath,
    /// The complete text discovery was given.
    pub text: String,
}

/// An ad-hoc discovery request together with what it was built from.
#[derive(Clone, Debug)]
pub struct CapturedSelection {
    /// The canonical directory the request's paths are relative to.
    pub development_root: PathBuf,
    /// The canonical selection root module names are measured from.
    pub selection_root: PathBuf,
    /// Every captured source, ordered by canonical path.
    pub sources: Vec<CapturedSource>,
    /// The request, with an empty CLI overlay for the caller to fill.
    pub request: DiscoveryRequest,
}

/// Captures an explicit source selection as an ad-hoc discovery request.
///
/// Each input contributes a root: a file its parent directory, a directory
/// itself. Every input must contribute the same root; a selection spanning
/// directories is rejected, because module names derive from the root and a
/// common ancestor would silently rename them. A directory contributes its
/// recursive contents that carry one of `file_extensions`; symbolic links
/// inside it are not followed. Every source is canonicalized with symbolic
/// links resolved, must stay under the root, is deduplicated by canonical
/// path, and is ordered by it. Only UTF-8 text is accepted.
///
/// With a manifest, the development root is the manifest's directory and the
/// selection must lie under it: the selection borrows that project's
/// identity, so it may not reach outside the project.
pub fn capture_source_selection(options: &SourceSelectionOptions<'_>) -> Result<CapturedSelection> {
    if options.inputs.is_empty() {
        bail!("workspace.selection.empty: no source inputs were given");
    }
    let current = std::env::current_dir().context("Failed to resolve the current directory")?;
    let mut root: Option<PathBuf> = None;
    let mut paths = BTreeSet::new();
    for input in options.inputs {
        let absolute = if input.is_absolute() {
            input.clone()
        } else {
            current.join(input)
        };
        let metadata = fs::metadata(&absolute).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => {
                anyhow!("Source input '{}' does not exist", absolute.display())
            }
            _ => anyhow!(
                "Failed to read source input '{}': {error}",
                absolute.display()
            ),
        })?;
        let canonical = fs::canonicalize(&absolute)
            .with_context(|| format!("Failed to resolve source input '{}'", absolute.display()))?;
        let input_root = if metadata.is_file() {
            canonical
                .parent()
                .ok_or_else(|| {
                    anyhow!(
                        "Source file '{}' has no parent directory",
                        absolute.display()
                    )
                })?
                .to_path_buf()
        } else if metadata.is_dir() {
            canonical.clone()
        } else {
            bail!(
                "Source input '{}' must be a file or directory",
                absolute.display()
            );
        };
        match &root {
            Some(existing) if existing != &input_root => bail!(
                "workspace.selection.spans-directories: source inputs resolve to different roots `{}` and `{}`; select sources from one directory",
                existing.display(),
                input_root.display()
            ),
            Some(_) => {}
            None => root = Some(input_root.clone()),
        }
        if metadata.is_file() {
            paths.insert(canonical);
        } else {
            collect_directory(&canonical, &canonical, options.file_extensions, &mut paths)?;
        }
    }
    let selection_root = root.expect("at least one input contributed a root");
    if paths.is_empty() {
        bail!(
            "The {} source set is empty below '{}'",
            options.language_id,
            selection_root.display()
        );
    }

    let (development_root, manifest) = match options.manifest {
        Some(manifest) => {
            let manifest = fs::canonicalize(manifest)
                .with_context(|| format!("Failed to resolve manifest '{}'", manifest.display()))?;
            let project = manifest
                .parent()
                .ok_or_else(|| {
                    anyhow!("Manifest '{}' has no parent directory", manifest.display())
                })?
                .to_path_buf();
            if !selection_root.starts_with(&project) {
                bail!(
                    "workspace.selection.outside-project: source root `{}` is not inside the project at `{}`",
                    selection_root.display(),
                    project.display()
                );
            }
            (project, Some(manifest))
        }
        None => (selection_root.clone(), None),
    };

    let mut used = 0_usize;
    let mut entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory)]);
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(&path)
            .with_context(|| format!("Failed to read source '{}'", path.display()))?;
        used = used.saturating_add(bytes.len());
        if used > options.byte_limit {
            bail!(
                "workspace.selection.resource-limit: the selected sources exceed {} bytes",
                options.byte_limit
            );
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            anyhow!(
                "workspace.selection.not-text: source '{}' is not UTF-8 text",
                path.display()
            )
        })?;
        let relative = relative_path(&development_root, &path)?;
        insert_parents(&mut entries, &relative)?;
        entries.insert(relative.clone(), FileEntry::File { text: text.clone() });
        sources.push(CapturedSource {
            path,
            relative,
            text,
        });
    }

    let project = match &manifest {
        Some(manifest) => {
            let text = fs::read_to_string(manifest)
                .with_context(|| format!("Failed to read manifest '{}'", manifest.display()))?;
            let relative = relative_path(&development_root, manifest)?;
            entries.insert(relative.clone(), FileEntry::File { text });
            ProjectSource::Manifest { path: relative }
        }
        None => ProjectSource::Synthesized,
    };
    let request = DiscoveryRequest {
        protocol_version: morphir_workspace::workspace_discovery_protocol(),
        development_root: FileTree { entries },
        morphir_home: None,
        system_config: None,
        environment: BTreeMap::new(),
        cli_overlay: serde_json::json!({}),
        purpose: DiscoveryPurpose::AdHocSources {
            project,
            sources: SourceSelection {
                root: relative_directory(&development_root, &selection_root)?,
                paths: sources
                    .iter()
                    .map(|source| source.relative.clone())
                    .collect(),
            },
            language_id: options.language_id.to_owned(),
        },
    };
    Ok(CapturedSelection {
        development_root,
        selection_root,
        sources,
        request,
    })
}

fn collect_directory(
    root: &Path,
    directory: &Path,
    extensions: &[String],
    paths: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let entries = fs::read_dir(directory)
        .with_context(|| format!("Failed to read source directory '{}'", directory.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| {
            format!("Failed to read source directory '{}'", directory.display())
        })?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("Failed to inspect '{}'", entry.path().display()))?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_directory(root, &path, extensions, paths)?;
        } else if file_type.is_file() && has_extension(&path, extensions) {
            let canonical = fs::canonicalize(&path)
                .with_context(|| format!("Failed to resolve source '{}'", path.display()))?;
            if !canonical.starts_with(root) {
                bail!(
                    "workspace.selection.outside-root: source '{}' resolves outside `{}`",
                    path.display(),
                    root.display()
                );
            }
            paths.insert(canonical);
        }
    }
    Ok(())
}

fn has_extension(path: &Path, extensions: &[String]) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    extensions.iter().any(|extension| {
        let extension = extension.trim_start_matches('.');
        name.rsplit_once('.')
            .is_some_and(|(stem, suffix)| !stem.is_empty() && suffix == extension)
    })
}

fn relative_directory(base: &Path, path: &Path) -> Result<RelativePath> {
    if base == path {
        Ok(RelativePath::root())
    } else {
        relative_path(base, path)
    }
}

fn relative_path(base: &Path, path: &Path) -> Result<RelativePath> {
    let relative = path.strip_prefix(base).map_err(|_| {
        anyhow!(
            "workspace.path.not-confined: `{}` is not under `{}`",
            path.display(),
            base.display()
        )
    })?;
    let segments = relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| anyhow!("Source path '{}' is not valid UTF-8", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    RelativePath::parse(segments.join("/"))
        .map_err(|error| anyhow!("workspace.path.not-confined: {error}"))
}

fn insert_parents(
    entries: &mut BTreeMap<RelativePath, FileEntry>,
    path: &RelativePath,
) -> Result<()> {
    let segments: Vec<&str> = path.as_str().split('/').collect();
    for end in 1..segments.len() {
        let parent = RelativePath::parse(segments[..end].join("/"))
            .map_err(|error| anyhow!("workspace.path.not-confined: {error}"))?;
        entries.entry(parent).or_insert(FileEntry::Directory);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, text: &str) -> PathBuf {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn options<'a>(inputs: &'a [PathBuf], extensions: &'a [String]) -> SourceSelectionOptions<'a> {
        SourceSelectionOptions {
            inputs,
            file_extensions: extensions,
            language_id: "elm",
            manifest: None,
            byte_limit: DEFAULT_SOURCE_BYTES,
        }
    }

    fn elm() -> Vec<String> {
        vec![".elm".to_owned()]
    }

    fn paths(captured: &CapturedSelection) -> (String, Vec<String>) {
        let DiscoveryPurpose::AdHocSources { sources, .. } = &captured.request.purpose else {
            panic!("an ad-hoc request");
        };
        (
            sources.root.as_str().to_owned(),
            sources
                .paths
                .iter()
                .map(|path| path.as_str().to_owned())
                .collect(),
        )
    }

    #[test]
    fn a_lone_file_is_a_selection_rooted_at_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "src/Widget.elm",
            "module Widget exposing (..)\n",
        );

        let captured = capture_source_selection(&options(&[file], &elm())).unwrap();

        assert_eq!(
            paths(&captured),
            (".".to_owned(), vec!["Widget.elm".to_owned()])
        );
        assert_eq!(captured.sources[0].text, "module Widget exposing (..)\n");
        assert_eq!(
            captured.selection_root,
            fs::canonicalize(dir.path().join("src")).unwrap()
        );
    }

    #[test]
    fn repeated_inputs_are_one_source_and_order_is_canonical() {
        let dir = tempfile::tempdir().unwrap();
        let b = write(dir.path(), "B.elm", "");
        let a = write(dir.path(), "A.elm", "");

        let captured = capture_source_selection(&options(&[b.clone(), a, b], &elm())).unwrap();

        assert_eq!(
            paths(&captured).1,
            vec!["A.elm".to_owned(), "B.elm".to_owned()]
        );
    }

    #[test]
    fn a_directory_collects_its_matching_contents_recursively() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/A.elm", "");
        write(dir.path(), "src/nested/B.elm", "");
        write(dir.path(), "src/notes.txt", "");

        let captured =
            capture_source_selection(&options(&[dir.path().join("src")], &elm())).unwrap();

        assert_eq!(
            paths(&captured),
            (
                ".".to_owned(),
                vec!["A.elm".to_owned(), "nested/B.elm".to_owned()]
            )
        );
    }

    #[test]
    fn inputs_spanning_directories_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "src/A.elm", "");
        let b = write(dir.path(), "lib/B.elm", "");

        let error = capture_source_selection(&options(&[a, b], &elm())).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("workspace.selection.spans-directories")
        );
    }

    #[test]
    fn a_missing_input_names_itself() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("Missing.elm");

        let error = capture_source_selection(&options(&[missing], &elm())).unwrap_err();

        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn an_empty_directory_is_an_empty_source_set() {
        let dir = tempfile::tempdir().unwrap();

        let error =
            capture_source_selection(&options(&[dir.path().to_path_buf()], &elm())).unwrap_err();

        assert!(error.to_string().contains("source set is empty"));
    }

    #[test]
    fn non_text_sources_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Binary.elm");
        fs::write(&file, [0xff, 0xfe, 0x00]).unwrap();

        let error = capture_source_selection(&options(&[file], &elm())).unwrap_err();

        assert!(error.to_string().contains("workspace.selection.not-text"));
    }

    #[test]
    fn the_byte_budget_bounds_the_selection() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "A.elm", "module A exposing (..)\n");
        let inputs = [file];
        let extensions = elm();
        let mut options = options(&inputs, &extensions);
        options.byte_limit = 4;

        let error = capture_source_selection(&options).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("workspace.selection.resource-limit")
        );
    }

    #[test]
    fn a_manifest_roots_the_request_at_its_project() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = write(
            dir.path(),
            "morphir.toml",
            "[project]\nname = 'acme/widgets'\n",
        );
        let file = write(dir.path(), "src/A.elm", "");
        let inputs = [file];
        let extensions = elm();
        let mut options = options(&inputs, &extensions);
        options.manifest = Some(&manifest);

        let captured = capture_source_selection(&options).unwrap();

        let DiscoveryPurpose::AdHocSources {
            project, sources, ..
        } = &captured.request.purpose
        else {
            panic!("an ad-hoc request");
        };
        assert_eq!(
            project,
            &ProjectSource::Manifest {
                path: RelativePath::parse("morphir.toml").unwrap()
            }
        );
        assert_eq!(sources.root.as_str(), "src");
        assert_eq!(sources.paths[0].as_str(), "src/A.elm");
        assert!(
            captured
                .request
                .development_root
                .contains_file(&RelativePath::parse("morphir.toml").unwrap())
        );
    }

    #[test]
    fn a_manifest_selection_may_not_leave_its_project() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = write(dir.path(), "project/morphir.toml", "");
        let file = write(dir.path(), "elsewhere/A.elm", "");
        let inputs = [file];
        let extensions = elm();
        let mut options = options(&inputs, &extensions);
        options.manifest = Some(&manifest);

        let error = capture_source_selection(&options).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("workspace.selection.outside-project")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_a_directory_input_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = write(dir.path(), "outside/Secret.elm", "");
        fs::create_dir_all(dir.path().join("src")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.path().join("src/Secret.elm")).unwrap();
        write(dir.path(), "src/A.elm", "");

        let captured =
            capture_source_selection(&options(&[dir.path().join("src")], &elm())).unwrap();

        // A link is not followed while walking a directory, so the linked
        // file never joins the selection.
        assert_eq!(paths(&captured).1, vec!["A.elm".to_owned()]);
    }
}
