//! Builtin extension registry.
//!
//! Provides discovery and access to all builtin extensions.

use crate::{BuiltinExtension, BuiltinInfo};
use std::collections::HashMap;

/// Registry of all available builtin extensions.
pub struct BuiltinRegistry {
    extensions: HashMap<String, Box<dyn BuiltinExtension>>,
}

impl BuiltinRegistry {
    /// Create a new registry with all available builtins.
    pub fn new() -> Self {
        let mut registry = Self {
            extensions: HashMap::new(),
        };

        // Register all builtins
        #[cfg(feature = "migrate")]
        {
            use crate::migrate::MigrateExtension;
            let ext = MigrateExtension;
            registry.register(Box::new(ext));
        }

        registry
    }

    /// Register a builtin extension.
    fn register(&mut self, extension: Box<dyn BuiltinExtension>) {
        let id = extension.info().id.clone();
        self.extensions.insert(id, extension);
    }

    /// Get a builtin extension by ID.
    pub fn get(&self, id: &str) -> Option<&dyn BuiltinExtension> {
        self.extensions.get(id).map(|b| b.as_ref())
    }

    /// List all available builtins.
    pub fn list(&self) -> Vec<BuiltinInfo> {
        self.extensions.values().map(|ext| ext.info()).collect()
    }

    /// Check if a builtin exists.
    pub fn contains(&self, id: &str) -> bool {
        self.extensions.contains_key(id)
    }
}

impl Default for BuiltinRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = BuiltinRegistry::new();
        let builtins = registry.list();

        #[cfg(feature = "migrate")]
        {
            assert!(!builtins.is_empty(), "Expected at least migrate builtin");
            assert!(registry.contains("migrate"), "Should contain migrate");
        }
    }

    #[test]
    #[cfg(feature = "migrate")]
    fn test_get_migrate() {
        let registry = BuiltinRegistry::new();
        let migrate = registry.get("migrate");
        assert!(migrate.is_some(), "Should find migrate builtin");
    }
}
