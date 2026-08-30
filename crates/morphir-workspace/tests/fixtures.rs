#![allow(dead_code)]

use std::{collections::BTreeMap, fs, path::Path};

use morphir_workspace::{
    DiscoveryRequest, FileEntry, FileTree, RelativePath, WORKSPACE_DISCOVERY_PROTOCOL,
};

pub(crate) fn fixture_request(name: &str) -> DiscoveryRequest {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/workspace-discovery")
        .join(name);
    let mut entries = BTreeMap::new();
    walk_fixture(&fixture_root, &fixture_root, &mut entries).unwrap();

    DiscoveryRequest {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        development_root: FileTree { entries },
        morphir_home: None,
        system_config: None,
        environment: BTreeMap::new(),
        cli_overlay: serde_json::Value::Null,
    }
}

fn walk_fixture(
    root: &Path,
    directory: &Path,
    entries: &mut BTreeMap<RelativePath, FileEntry>,
) -> Result<(), String> {
    let mut children = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    children.sort();

    for child in children {
        let relative = child.strip_prefix(root).unwrap();
        let relative = relative
            .components()
            .map(|component| component.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/");
        let relative = RelativePath::parse(relative).unwrap();
        let metadata = fs::symlink_metadata(&child).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "fixture contains unsupported symlink `{}`",
                relative.as_str()
            ));
        }
        if metadata.is_dir() {
            entries.insert(relative, FileEntry::Directory);
            walk_fixture(root, &child, entries)?;
        } else if metadata.is_file() {
            entries.insert(
                relative,
                FileEntry::File {
                    text: fs::read_to_string(child).map_err(|error| error.to_string())?,
                },
            );
        } else {
            return Err(format!(
                "fixture contains unsupported file type `{}`",
                relative.as_str()
            ));
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        os::unix::fs::symlink,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::walk_fixture;

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn fixture_walker_rejects_directory_symlink_before_recursion() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "morphir-workspace-fixture-symlink-{}-{nonce}",
            process::id()
        ));
        let cleanup = TestDirectory(base.clone());
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("must-not-read.txt"), "outside fixture").unwrap();
        symlink(&outside, root.join("linked")).unwrap();
        let mut entries = BTreeMap::new();

        let error = walk_fixture(&root, &root, &mut entries).unwrap_err();

        assert_eq!(error, "fixture contains unsupported symlink `linked`");
        assert!(entries.is_empty());
        drop(cleanup);
    }
}
