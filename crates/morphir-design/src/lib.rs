pub mod config;
pub mod extensions;

pub use config::{
    ConfigContext, discover_config, discover_morphir_dir, load_config_context,
    resolve_compile_output, resolve_generate_output, resolve_dist_output,
    resolve_test_fixture, resolve_test_scenario, sanitize_project_name,
    resolve_path_relative_to_config, resolve_path_relative_to_workspace,
    ensure_morphir_structure,
};
pub use extensions::{
    BuiltinExtension, ExtensionInfo, ExtensionSource,
    discover_builtin_extensions, get_builtin_extension_path, resolve_extension_source,
};
