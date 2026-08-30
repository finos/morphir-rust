use std::{collections::BTreeMap, fs};

use morphir_workspace::{FileEntry, FileTree, RelativePath};

#[cfg(unix)]
use std::path::Path;

#[cfg(unix)]
use cap_std::{ambient_authority, fs::Dir};

use super::{
    aliases::{
        AliasBudgets, DirectoryAlias, materialize_directory_aliases, record_directory_alias,
    },
    budget::{PayloadBudget, PayloadKind, entry_bytes},
    mounts::{apply_user_override_selection, selected_mount},
    traversal::{BoundaryEvent, TraversalBudgets, build_tree_with},
    *,
};
use crate::config::sources::SourceSelection;

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
fn open_capability(path: &Path) -> Dir {
    Dir::open_ambient_dir(path, ambient_authority()).unwrap()
}

fn payload_for_entries(entries: &BTreeMap<RelativePath, FileEntry>) -> PayloadBudget {
    let mut payload = PayloadBudget::new(TraversalBudgets::DEFAULT.config_bytes);
    for entry in entries.values() {
        payload.reserve(entry_bytes(entry)).unwrap();
    }
    payload
}
mod aliases;
mod budgets;
mod mounts;
mod traversal;
