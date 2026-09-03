//! Path helpers relative to configuration files and the .morphir support directory.

use super::discovery::config_root;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Resolve test fixture path
pub fn resolve_test_fixture(name: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir.join("test").join("fixtures").join(name)
}

/// Resolve test scenario path
pub fn resolve_test_scenario(name: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir.join("test").join("scenarios").join(name)
}

/// Resolve path relative to config file location
pub fn resolve_path_relative_to_config(path: &Path, config_path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_root(config_path)
            .unwrap_or(Path::new("."))
            .join(path)
    }
}

/// Resolve path relative to workspace root
pub fn resolve_path_relative_to_workspace(path: &Path, workspace_root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

/// Ensure .morphir/ folder structure is created
pub fn ensure_morphir_structure(morphir_dir: &Path) -> Result<()> {
    // Create base directories
    std::fs::create_dir_all(morphir_dir.join("test").join("fixtures"))?;
    std::fs::create_dir_all(morphir_dir.join("test").join("scenarios"))?;
    std::fs::create_dir_all(morphir_dir.join("logs"))?;
    std::fs::create_dir_all(morphir_dir.join("cache"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_against_the_project_root() {
        let hidden_config = Path::new("/p/.morphir/morphir.yaml");
        assert_eq!(
            resolve_path_relative_to_config(Path::new("src"), hidden_config),
            PathBuf::from("/p/src")
        );
        assert_eq!(
            resolve_path_relative_to_workspace(Path::new("packages/a"), Path::new("/ws")),
            PathBuf::from("/ws/packages/a")
        );
    }

    #[test]
    fn ensure_structure_creates_support_directories_but_not_out() {
        let root = tempfile::tempdir().unwrap();
        let morphir_dir = root.path().join(".morphir");

        ensure_morphir_structure(&morphir_dir).unwrap();

        for sub in ["test/fixtures", "test/scenarios", "logs", "cache"] {
            assert!(morphir_dir.join(sub).is_dir(), "missing {sub}");
        }
        assert!(
            !morphir_dir.join("out").exists(),
            "out/ belongs to the out root"
        );
    }
}
