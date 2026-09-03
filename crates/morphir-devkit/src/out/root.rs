//! Out root resolution: flag, environment, config, default.

use crate::config::ConfigContext;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Default out directory relative to the workspace root.
pub const DEFAULT_OUT_DIR: &str = ".morphir/out";

/// Resolve the out root. Precedence: `flag`, then `env`, then
/// `[workspace].out_dir`, then `DEFAULT_OUT_DIR` under the workspace root.
/// `flag` and `env` resolve against `cwd`. Config values resolve against the
/// workspace root, or the configuration directory outside a workspace.
/// Without a configuration the default resolves against `cwd`.
pub fn resolve_out_root(
    flag: Option<&Path>,
    env: Option<&OsStr>,
    context: Option<&ConfigContext>,
    cwd: &Path,
) -> PathBuf {
    if let Some(flag) = flag {
        return absolute(cwd, flag);
    }
    if let Some(env) = env.filter(|value| !value.is_empty()) {
        return absolute(cwd, Path::new(env));
    }
    let Some(context) = context else {
        return cwd.join(DEFAULT_OUT_DIR);
    };
    let base = workspace_base(context);
    let configured = context
        .config
        .workspace
        .as_ref()
        .map(|workspace| workspace.out_dir.as_str())
        .unwrap_or(DEFAULT_OUT_DIR);
    absolute(&base, Path::new(configured))
}

/// The module's path relative to the workspace root. Empty for the root
/// module or outside a workspace.
pub fn module_path(context: &ConfigContext) -> PathBuf {
    match (&context.workspace_root, &context.project_root) {
        (Some(workspace), Some(project)) => project
            .strip_prefix(workspace)
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        _ => PathBuf::new(),
    }
}

fn workspace_base(context: &ConfigContext) -> PathBuf {
    context
        .workspace_root
        .clone()
        .or_else(|| context.project_root.clone())
        .or_else(|| context.config_path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn absolute(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config_context;
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn flag_beats_env_beats_config_beats_default() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("morphir.toml");
        write(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n\n[workspace]\nout_dir = \"build/out\"\n",
        );
        let context = load_config_context(&config).unwrap();
        let cwd = temp.path().join("elsewhere");

        assert_eq!(
            resolve_out_root(
                Some(Path::new("flag")),
                Some(OsStr::new("env")),
                Some(&context),
                &cwd
            ),
            cwd.join("flag")
        );
        assert_eq!(
            resolve_out_root(None, Some(OsStr::new("env")), Some(&context), &cwd),
            cwd.join("env")
        );
        assert_eq!(
            resolve_out_root(None, None, Some(&context), &cwd),
            temp.path().join("build/out")
        );
    }

    #[test]
    fn default_is_dot_morphir_out_under_the_workspace_root() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("morphir.toml");
        write(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n",
        );
        let context = load_config_context(&config).unwrap();
        assert_eq!(
            resolve_out_root(None, None, Some(&context), Path::new("/somewhere")),
            temp.path().join(".morphir").join("out")
        );
    }

    /// A member never gets its own out root, whichever of the two
    /// configuration files the caller discovered. Loading the workspace
    /// configuration selects the single member; loading the member's own
    /// configuration finds the workspace by walking up. Both must land on the
    /// same workspace-level root and the same module path.
    #[test]
    fn members_resolve_to_the_workspace_out_root() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("morphir.toml");
        write(&workspace, "[workspace]\nmembers = [\"packages/*\"]\n");
        let member = temp.path().join("packages/orders/morphir.toml");
        write(
            &member,
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );
        // `cwd` is deliberately NOT the workspace root (it sits beside it,
        // under a `scratch` directory) so these assertions can only pass if
        // the workspace root from `context` actually drove the resolution.
        // Using `cwd == workspace_root` here would let a config-ignoring
        // implementation that falls through to `cwd.join(DEFAULT_OUT_DIR)`
        // produce the same path and pass by coincidence.
        let cwd = temp.path().join("scratch");
        for entry in [&workspace, &member] {
            let context = load_config_context(entry).unwrap();
            assert_eq!(
                resolve_out_root(None, None, Some(&context), &cwd),
                temp.path().join(".morphir").join("out"),
                "loaded from {}",
                entry.display()
            );
            assert_eq!(
                module_path(&context),
                PathBuf::from("packages/orders"),
                "loaded from {}",
                entry.display()
            );
        }
    }

    #[test]
    fn an_empty_env_value_is_ignored_in_favor_of_config() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("morphir.toml");
        write(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n\n[workspace]\nout_dir = \"build/out\"\n",
        );
        let context = load_config_context(&config).unwrap();
        // `cwd` is deliberately NOT the config's directory, so a bug that
        // treated the empty environment variable as present (falling
        // through to `absolute(cwd, "")`) would resolve under `cwd` instead
        // of under the configured `[workspace].out_dir`, and the assertion
        // below would catch it.
        let cwd = temp.path().join("elsewhere");
        assert_eq!(
            resolve_out_root(None, Some(OsStr::new("")), Some(&context), &cwd),
            temp.path().join("build/out")
        );
    }

    #[test]
    fn root_module_path_is_empty() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("morphir.toml");
        write(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n",
        );
        let context = load_config_context(&config).unwrap();
        assert_eq!(module_path(&context), PathBuf::new());
    }

    /// A context loaded from a relative config path stores absolute roots
    /// (see `loader::tests::relative_config_path_is_stored_absolute`), so
    /// `resolve_out_root` must land under the config's own directory even
    /// when the caller's `cwd` at resolution time is somewhere else
    /// entirely. The config is written directly under the test binary's
    /// current directory (never changed by this test) so a genuinely
    /// relative path can be passed without touching global process state.
    #[test]
    fn resolve_out_root_ignores_cwd_when_the_context_was_loaded_from_a_relative_path() {
        let cwd = std::env::current_dir().unwrap();
        let relative_dir = PathBuf::from(format!(
            ".tmp-resolve-out-root-relative-context-{}",
            std::process::id()
        ));
        let dir = cwd.join(&relative_dir);
        let config = dir.join("morphir.toml");
        write(
            &config,
            "[project]\nname = \"acme/app\"\nversion = \"1.0.0\"\n",
        );

        let relative_config = relative_dir.join("morphir.toml");
        let context_result = load_config_context(&relative_config);
        std::fs::remove_dir_all(&dir).unwrap();
        let context = context_result.unwrap();

        // A cwd unrelated to either the test binary's directory or the
        // config's directory: if `resolve_out_root` resolved against it, the
        // result would land here instead of under `dir`.
        let unrelated_cwd = std::env::temp_dir().join(format!(
            "resolve-out-root-unrelated-cwd-{}",
            std::process::id()
        ));

        assert_eq!(
            resolve_out_root(None, None, Some(&context), &unrelated_cwd),
            dir.join(".morphir").join("out")
        );
    }

    #[test]
    fn without_config_the_root_is_under_cwd() {
        assert_eq!(
            resolve_out_root(None, None, None, Path::new("/scratch")),
            PathBuf::from("/scratch/.morphir/out")
        );
        assert_eq!(
            resolve_out_root(
                None,
                Some(OsStr::new("/abs/out")),
                None,
                Path::new("/scratch")
            ),
            PathBuf::from("/abs/out")
        );
    }
}
