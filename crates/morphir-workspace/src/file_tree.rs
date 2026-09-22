use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::RelativePath;

/// The current version of the workspace discovery wire protocol.
pub const WORKSPACE_DISCOVERY_PROTOCOL: u32 = 1;

/// An entry in a portable, root-confined file tree.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FileEntry {
    /// A directory.
    Directory,
    /// A UTF-8 text file.
    File {
        /// The complete text file contents.
        text: String,
    },
    /// A symbolic link whose target is confined to the same named mount.
    ///
    /// Protocol-v1 discovery rejects unresolved link entries explicitly. Hosts
    /// must materialize confined targets into directory and file entries before
    /// invoking discovery.
    Symlink {
        /// The canonical target path under the named mount.
        target: RelativePath,
    },
}

/// A deterministic collection of canonical paths and their entries.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileTree {
    /// Entries ordered by canonical relative path.
    pub entries: BTreeMap<RelativePath, FileEntry>,
}

impl FileTree {
    /// Returns whether `path` names a UTF-8 text file in this tree.
    #[must_use]
    pub fn contains_file(&self, path: &RelativePath) -> bool {
        matches!(self.entries.get(path), Some(FileEntry::File { .. }))
    }

    /// Returns the text stored at `path`, or `None` when it is not a text file.
    #[must_use]
    pub fn file_text(&self, path: &RelativePath) -> Option<&str> {
        match self.entries.get(path) {
            Some(FileEntry::File { text }) => Some(text),
            _ => None,
        }
    }

    /// Iterates over canonical directory paths in sorted order.
    pub fn directories(&self) -> impl Iterator<Item = &RelativePath> {
        self.entries
            .iter()
            .filter_map(|(path, entry)| matches!(entry, FileEntry::Directory).then_some(path))
    }
}

/// A complete, provider-independent request for workspace discovery.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryRequest {
    /// The requested workspace discovery protocol version.
    pub protocol_version: u32,
    /// The file tree rooted at the development mount.
    pub development_root: FileTree,
    /// The optional file tree rooted at the Morphir home mount.
    pub morphir_home: Option<FileTree>,
    /// The optional file tree rooted at the system configuration mount.
    pub system_config: Option<FileTree>,
    /// Environment values available to portable configuration resolution.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Command-line configuration values overlaid onto discovered configuration.
    ///
    /// For [`DiscoveryPurpose::AdHocSources`], `project.name` here is the
    /// *only* place an explicit project name can come from: a host that wants
    /// to name a synthesized project puts it at `cli_overlay.project.name`.
    /// Portable discovery reads that exact path directly, never the merged
    /// effective configuration, so a default from built-in defaults, a shared
    /// system/global layer or the environment can never be mistaken for an
    /// explicit override. When absent, the synthesized project has no name
    /// and discovery requires its selection to contain exactly one source.
    #[serde(default)]
    pub cli_overlay: serde_json::Value,
    /// What this request is asking for.
    #[serde(default)]
    pub purpose: DiscoveryPurpose,
}

/// Which sources a request selects, and the root they are measured from.
///
/// The root is carried, never recomputed. Module names are derived relative to
/// it, and recovering it from the files gets it wrong: a selection of
/// `models/domain/customer.gleam` has no `src` segment, so a provider guessing
/// a root falls through to the basename and names the module `customer`
/// instead of `domain/customer`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSelection {
    /// The root the selected paths are measured from, relative to the
    /// development root.
    pub root: RelativePath,
    /// The selected sources, relative to the development root, in the order
    /// the caller gave them.
    pub paths: Vec<RelativePath>,
}

/// Where a request's project identity comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ProjectSource {
    /// No manifest. Name, version and configuration are synthesized.
    Synthesized,
    /// Identity comes from the manifest at this path, relative to the
    /// development root, while the selection decides the sources.
    #[serde(rename_all = "camelCase")]
    Manifest { path: RelativePath },
}

/// What a discovery request is asking for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DiscoveryPurpose {
    /// Find the projects this tree's manifests describe. The default, and what
    /// discovery has always meant.
    #[default]
    ManifestProjects,
    /// Compile an explicit selection of sources, whose identity comes from
    /// `project`.
    #[serde(rename_all = "camelCase")]
    AdHocSources {
        project: ProjectSource,
        sources: SourceSelection,
        /// The language every selected source is in. One language per set.
        /// Discovery rejects an empty string; it does not otherwise validate
        /// the value.
        language_id: String,
    },
}
