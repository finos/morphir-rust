pub mod config;
pub mod extensions;

pub use config::{
    discover_config, discover_morphir_dir, ensure_morphir_structure, load_config_context,
    resolve_compile_output, resolve_dist_output, resolve_generate_output,
    resolve_path_relative_to_config, resolve_path_relative_to_workspace, resolve_test_fixture,
    resolve_test_scenario, sanitize_project_name, ConfigContext,
};
pub use extensions::{
    discover_builtin_extensions, get_builtin_extension_path, resolve_extension_source,
    BuiltinExtension, ExtensionInfo, ExtensionSource,
};
