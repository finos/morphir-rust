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
pub mod paths;
mod provenance;
pub mod sources;

pub use discovery::{
    ConfigLayout, ConfigPlatform, config_layout, config_root, default_system_config_dir,
    discover_config, discover_config_at, discover_config_candidates, discover_global_config,
    discover_morphir_dir, discover_system_config, discover_user_override, global_config_candidates,
    system_config_candidates, user_override_candidates,
};
pub use loader::{
    ConfigContext, builtin_defaults, load_config_context, load_config_context_with,
    load_config_context_with_global, load_effective_config,
};
pub use paths::{
    ensure_morphir_structure, resolve_compile_output, resolve_dist_output, resolve_generate_output,
    resolve_path_relative_to_config, resolve_path_relative_to_workspace, resolve_test_fixture,
    resolve_test_scenario, sanitize_project_name,
};
pub use sources::{
    ConfigLoadOptions, ConfigSource, ConfigSourceKind, ConfigSourceStatus, EffectiveConfig,
    EnvSelection, SourceSelection,
};
