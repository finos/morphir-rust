//! Decorator Support for IR Transformations
//!
//! Decorators are sidecar files (.deco.json) that provide metadata and
//! transformation hints for IR nodes.

use crate::pipeline::Step;
use crate::Result;
use morphir_ir::naming::FQName;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Decorator registry mapping FQNames to decorator metadata
#[derive(Debug, Clone, Default)]
pub struct DecoratorRegistry {
    decorations: HashMap<FQName, Value>,
}

impl DecoratorRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            decorations: HashMap::new(),
        }
    }

    /// Get decorations for an FQName
    pub fn get_decorations(&self, fqname: &FQName) -> Option<&Value> {
        self.decorations.get(fqname)
    }

    /// Check if an FQName has a specific decorator type
    pub fn has_decorator(&self, fqname: &FQName, decorator_type: &str) -> bool {
        self.decorations
            .get(fqname)
            .and_then(|deco| deco.get("decorations"))
            .and_then(|deco| deco.get(decorator_type))
            .is_some()
    }

    /// Add a decoration
    pub fn add_decoration(&mut self, fqname: FQName, decoration: Value) {
        self.decorations.insert(fqname, decoration);
    }
}

/// Decorator loader step
pub struct DecoratorLoader {
    deco_dir: PathBuf,
}

impl DecoratorLoader {
    pub fn new(deco_dir: PathBuf) -> Self {
        Self { deco_dir }
    }

    fn load_decorators(&self) -> Result<DecoratorRegistry> {
        use crate::vfs::{OsVfs, Vfs};
        use std::fs;

        let vfs = OsVfs;
        let mut registry = DecoratorRegistry::new();

        // Walk deco directory and load all .deco.json files
        if vfs.exists(&self.deco_dir) && vfs.is_dir(&self.deco_dir) {
            if let Ok(entries) = fs::read_dir(&self.deco_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && path.extension().and_then(|s| s.to_str()) == Some("deco.json")
                    {
                        if let Ok(content) = vfs.read_to_string(&path) {
                            if let Ok(deco) = serde_json::from_str::<Value>(&content) {
                                // Extract target FQName from decorator
                                if let Some(target) = deco.get("target").and_then(|t| t.as_str()) {
                                    // Parse FQName (format: "package:module#name")
                                    if let Some(fqname) = parse_fqname_from_string(target) {
                                        registry.add_decoration(fqname, deco);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(registry)
    }
}

/// Parse FQName from string format "package:module#name"
fn parse_fqname_from_string(s: &str) -> Option<FQName> {
    use morphir_ir::naming::{Name, PackageName, Path};

    // Split by #
    let parts: Vec<&str> = s.split('#').collect();
    if parts.len() != 2 {
        return None;
    }

    let (package_module, name) = (parts[0], parts[1]);

    // Split package:module
    let pm_parts: Vec<&str> = package_module.split(':').collect();
    if pm_parts.len() != 2 {
        return None;
    }

    let package_path = Path::new(pm_parts[0]);
    let module_path = Path::new(pm_parts[1]);
    let local_name = Name::from(name);

    Some(FQName {
        package_path: PackageName::new(package_path).into(),
        module_path: module_path.into(),
        local_name,
    })
}

impl Step for DecoratorLoader {
    type Base = ();
    type Input = Value;
    type Output = (Value, DecoratorRegistry);

    fn run(&self, input: Self::Input) -> Result<Self::Output> {
        let registry = self.load_decorators()?;
        Ok((input, registry))
    }
}

/// Decorator applier step
pub struct DecoratorApplier;

impl Step for DecoratorApplier {
    type Base = ();
    type Input = (Value, DecoratorRegistry);
    type Output = Value;

    fn run(&self, (ir, _decorators): Self::Input) -> Result<Self::Output> {
        // For now, pass through - decorator application logic can be added later
        // Decorators can modify IR structure, add metadata, etc.
        Ok(ir)
    }
}
