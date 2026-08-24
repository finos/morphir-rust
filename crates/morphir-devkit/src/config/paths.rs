//! Output-path resolution inside the `.morphir/` directory and path helpers
//! relative to configuration files.

use super::discovery::config_root;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Resolve compile output path using Mill-inspired structure
pub fn resolve_compile_output(project: &str, language: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("compile")
        .join(language)
}

/// Resolve generate output path using Mill-inspired structure
pub fn resolve_generate_output(project: &str, target: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("generate")
        .join(target)
}

/// Resolve distribution output path
pub fn resolve_dist_output(project: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir
        .join("out")
        .join(sanitize_project_name(project))
        .join("dist")
}

/// Resolve test fixture path
pub fn resolve_test_fixture(name: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir.join("test").join("fixtures").join(name)
}

/// Resolve test scenario path
pub fn resolve_test_scenario(name: &str, morphir_dir: &Path) -> PathBuf {
    morphir_dir.join("test").join("scenarios").join(name)
}

/// Sanitize project name for filesystem use
pub fn sanitize_project_name(name: &str) -> String {
    // Replace invalid characters, but preserve structure
    // For now, just replace slashes and spaces
    name.replace(['/', ' ', '\\'], "-")
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
    std::fs::create_dir_all(morphir_dir.join("out"))?;
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
    fn output_paths_use_sanitized_project_names() {
        let morphir_dir = Path::new("/ws/.morphir");
        assert_eq!(
            resolve_compile_output("acme/orders", "gleam", morphir_dir),
            morphir_dir.join("out/acme-orders/compile/gleam")
        );
        assert_eq!(
            resolve_generate_output("acme orders", "scala", morphir_dir),
            morphir_dir.join("out/acme-orders/generate/scala")
        );
        assert_eq!(
            resolve_dist_output(r"acme\orders", morphir_dir),
            morphir_dir.join("out/acme-orders/dist")
        );
    }

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
    fn ensure_structure_creates_every_directory() {
        let root = tempfile::tempdir().unwrap();
        let morphir_dir = root.path().join(".morphir");

        ensure_morphir_structure(&morphir_dir).unwrap();

        for sub in ["out", "test/fixtures", "test/scenarios", "logs", "cache"] {
            assert!(morphir_dir.join(sub).is_dir(), "missing {sub}");
        }
    }
}
