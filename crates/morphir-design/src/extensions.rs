use std::path::PathBuf;

/// Information about a builtin extension
#[derive(Debug, Clone)]
pub struct BuiltinExtension {
    /// Extension identifier (e.g., "gleam")
    pub id: String,
    /// Display name
    pub name: String,
    /// Path to WASM file (if bundled)
    pub path: Option<PathBuf>,
    /// Supported languages (for frontend extensions)
    pub languages: Vec<String>,
    /// Supported targets (for backend extensions)
    pub targets: Vec<String>,
}

/// Discover builtin extensions from CLI resources
pub fn discover_builtin_extensions() -> Vec<BuiltinExtension> {
    let mut extensions = Vec::new();

    // Gleam binding is a builtin extension
    extensions.push(BuiltinExtension {
        id: "gleam".to_string(),
        name: "Gleam Language Binding".to_string(),
        path: get_builtin_extension_path("gleam"),
        languages: vec!["gleam".to_string()],
        targets: vec!["gleam".to_string()],
    });

    extensions
}

/// Get the path to a builtin extension WASM file
pub fn get_builtin_extension_path(extension_id: &str) -> Option<PathBuf> {
    // Check for bundled resources in multiple locations
    let possible_paths = vec![
        // Bundled in CLI binary resources (relative to executable)
        get_executable_dir().map(|dir| {
            dir.join("extensions")
                .join(format!("{}.wasm", extension_id))
        }),
        // In resources directory (relative to executable)
        get_executable_dir().map(|dir| {
            dir.join("resources")
                .join("extensions")
                .join(format!("{}.wasm", extension_id))
        }),
        // In the same directory as the binary
        get_executable_dir().map(|dir| dir.join(format!("{}.wasm", extension_id))),
        // Embedded in binary (using include_bytes! in build.rs)
        // This would require build-time code generation
    ];

    for path_opt in possible_paths {
        if let Some(path) = path_opt {
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

/// Get the directory containing the executable
fn get_executable_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
}

/// Extension information for discovery
#[derive(Debug, Clone)]
pub struct ExtensionInfo {
    /// Extension identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Extension source type
    pub source: ExtensionSource,
    /// Supported languages (for frontend)
    pub languages: Vec<String>,
    /// Supported targets (for backend)
    pub targets: Vec<String>,
}

/// Extension source type
#[derive(Debug, Clone)]
pub enum ExtensionSource {
    /// Builtin extension (bundled with CLI)
    Builtin { path: Option<PathBuf> },
    /// Extension from registry
    Registry { location: String },
    /// Extension from config
    Config { path: PathBuf },
}

/// Resolve extension source from config and builtin paths
pub fn resolve_extension_source(
    config: &Option<morphir_common::config::model::ExtensionSpec>,
    builtin_path: Option<PathBuf>,
) -> ExtensionSource {
    if let Some(builtin) = builtin_path {
        ExtensionSource::Builtin {
            path: Some(builtin),
        }
    } else if let Some(ext_config) = config {
        // Check if config specifies a path
        if let Some(path) = &ext_config.path {
            ExtensionSource::Config { path: path.clone() }
        } else {
            // Default to registry
            ExtensionSource::Registry {
                location: "registry".to_string(),
            }
        }
    } else {
        ExtensionSource::Registry {
            location: "registry".to_string(),
        }
    }
}
