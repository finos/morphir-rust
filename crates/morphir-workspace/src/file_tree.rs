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
    #[serde(default)]
    pub cli_overlay: serde_json::Value,
}
