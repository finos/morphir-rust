use super::*;

use std::collections::BTreeMap;

use morphir_workspace::{
    DiscoveryRequest, FileEntry, FileTree, ProjectState, RelativePath, WORKSPACE_DISCOVERY_PROTOCOL,
};

use crate::{
    ConfigLoadOptions, SourceSelection, build_workspace_discovery_request, discover_workspace,
};

fn write_file(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().expect("config parent")).unwrap();
    std::fs::write(path, content).unwrap();
}

fn write_project_config(path: &Path) {
    write_file(path, "project:\n  name: Acme.Project\n  version: 1.0.0\n");
}

fn workspace_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/workspace-discovery/valid-monorepo")
}

fn fixture_request() -> DiscoveryRequest {
    fn walk(root: &Path, directory: &Path, entries: &mut BTreeMap<RelativePath, FileEntry>) {
        let mut children = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            let relative = child
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            let relative = RelativePath::parse(relative).unwrap();
            if child.is_dir() {
                entries.insert(relative, FileEntry::Directory);
                walk(root, &child, entries);
            } else {
                entries.insert(
                    relative,
                    FileEntry::File {
                        text: std::fs::read_to_string(&child).unwrap(),
                    },
                );
            }
        }
    }

    let root = workspace_fixture_root();
    let mut entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory)]);
    walk(&root, &root, &mut entries);
    DiscoveryRequest {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        development_root: FileTree { entries },
        morphir_home: None,
        system_config: None,
        environment: BTreeMap::new(),
        cli_overlay: serde_json::json!({}),
    }
}

mod environment;
mod legacy;
mod overrides;
mod portable;
mod symlinks;
