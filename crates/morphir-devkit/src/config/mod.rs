//! Configuration discovery, layered loading, and workspace/project context.
//!
//! - [`discovery`]: where configuration files live and how to find exactly one
//!   per location (project, global user, system, user override).
//! - [`sources`]: the source kinds, their precedence, caller selections, and
//!   the per-source report produced by the loader.
//! - [`loader`]: merges every source into the effective configuration and
//!   resolves the surrounding workspace/project context.
//! - [`paths`]: output-path resolution inside `.morphir/`.

pub mod discovery;
pub mod loader;
mod members;
pub mod paths;
mod provenance;
pub mod secret;
pub mod sources;
mod workspace_discovery;

pub use discovery::{
    ConfigLayout, ConfigPlatform, config_layout, config_root, default_system_config_dir,
    discover_config, discover_config_at, discover_config_candidates, discover_global_config,
    discover_morphir_dir, discover_system_config, discover_user_override, global_config_candidates,
    system_config_candidates, user_override_candidates,
};
pub use loader::{
    ConfigContext, deprecated_key_warnings, load_config_context, load_config_context_with,
    load_config_context_with_global, load_effective_config,
};
pub use morphir_config::builtin_defaults;
pub use paths::{
    ensure_morphir_structure, resolve_path_relative_to_config, resolve_path_relative_to_workspace,
    resolve_test_fixture, resolve_test_scenario,
};
pub use secret::{
    ExposeSecret, SecretReference, SecretResolutionContext, SecretResolutionError, SecretResolver,
    SecretString, SystemSecretResolver,
};
pub use sources::{
    ConfigLoadOptions, ConfigSource, ConfigSourceKind, ConfigSourceStatus, EffectiveConfig,
    EnvSelection, SourceSelection,
};
pub use workspace_discovery::{
    NativeWorkspaceDiscovery, NativeWorkspaceDiscoveryError, build_workspace_discovery_request,
    discover_workspace, discover_workspace_detailed, discover_workspace_detailed_typed,
};

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    /// Make the current test executable available at `destination` so a test
    /// can exec it as a helper program.
    ///
    /// On Unix this must NOT copy: a copy opens the destination for writing,
    /// and any concurrently spawning test forks a child that briefly inherits
    /// that write descriptor, making a subsequent exec of the helper fail
    /// with `ETXTBSY`. A symlink never opens the file, so no such window
    /// exists. Windows has no `ETXTBSY` and restricts symlink creation, so a
    /// copy is used there.
    pub(crate) fn install_helper_executable(destination: &Path) {
        let source = std::env::current_exe().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, destination).unwrap();
        #[cfg(not(unix))]
        {
            std::fs::copy(&source, destination).unwrap();
        }
    }
}
