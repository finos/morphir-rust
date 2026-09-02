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

    // `load_config_context` only populates `workspace_root` when the config
    // path handed to it is itself the workspace's primary config (it checks
    // whether the decoded root config has a `[workspace]` section). Calling
    // it directly on a member's own config file (as the original brief
    // sketch did) leaves `workspace_root` `None`, so neither property this
    // test needs to prove would hold. Mirroring
    // `workspace_member_layers_sit_between_project_and_user_override` in
    // `config/loader.rs`, this loads the *workspace* config path with an
    // explicit member entry; the loader then selects and merges that member,
    // populating both `workspace_root` and `project_root`. Member globs
    // (`packages/*`) are also not supported yet -- `merge_workspace_member`
    // treats glob patterns literally -- so an exact member path is used.
    #[test]
    fn members_resolve_to_the_workspace_out_root() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("morphir.toml");
        write(&workspace, "[workspace]\nmembers = [\"packages/orders\"]\n");
        let member = temp.path().join("packages/orders/morphir.toml");
        write(
            &member,
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );
        let context = load_config_context(&workspace).unwrap();
        // `cwd` is deliberately NOT the workspace root (it sits beside it,
        // under a `scratch` directory) so this assertion can only pass if
        // the workspace root from `context` actually drove the resolution.
        // Using `cwd == workspace_root` here would let a config-ignoring
        // implementation that falls through to `cwd.join(DEFAULT_OUT_DIR)`
        // produce the same path and pass by coincidence.
        let cwd = temp.path().join("scratch");
        assert_eq!(
            resolve_out_root(None, None, Some(&context), &cwd),
            temp.path().join(".morphir").join("out")
        );
        assert_eq!(module_path(&context), PathBuf::from("packages/orders"));
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
