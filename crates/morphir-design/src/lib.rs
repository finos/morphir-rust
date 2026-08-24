pub mod config;
pub mod extensions;

pub use config::{
    ConfigContext, ConfigPlatform, discover_config, discover_config_candidates,
    discover_global_config, discover_morphir_dir, ensure_morphir_structure,
    global_config_candidates, load_config_context, load_config_context_with_global,
    resolve_compile_output, resolve_dist_output, resolve_generate_output,
    resolve_path_relative_to_config, resolve_path_relative_to_workspace, resolve_test_fixture,
    resolve_test_scenario, sanitize_project_name,
};
pub use extensions::{
    BuiltinExtension, ExtensionInfo, ExtensionSource, discover_builtin_extensions,
    get_builtin_extension_path, resolve_extension_source,
};
