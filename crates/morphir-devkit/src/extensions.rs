use std::path::PathBuf;

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
