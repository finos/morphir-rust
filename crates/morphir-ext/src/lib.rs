use wasmtime::component::{bindgen, Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView, WasiCtxView};
use std::collections::HashMap;
use std::path::Path;
use anyhow::Result;

// Re-export core types
pub use morphir_ext_core::{Envelope, Header, EnvValue as CoreEnvValue, LogLevel as CoreLogLevel};

// Generate bindings for wasmtime 41.0
bindgen!({
    world: "extension",
    path: "../morphir-ext-core/wit",
});

// Re-export generated types
pub use morphir::ext::envelope::{Envelope as WitEnvelope, Header as WitHeader};
pub use morphir::ext::runtime::{EnvValue, LogLevel};

/// Host state for the extension, supporting WASI and Morphir Runtime.
pub struct MorphirHost {
    /// WASI context (file descriptors, environment, etc.)
    ctx: WasiCtx,
    /// Component Resource Table
    table: ResourceTable,
    /// In-memory environment variables (for get/set-env-var)
    env_vars: HashMap<String, EnvValue>,
}

impl MorphirHost {
    pub fn new() -> Self {
        let ctx = WasiCtxBuilder::new()
            .inherit_stdio()
            .build();
        
        Self {
            ctx,
            table: ResourceTable::new(),
            env_vars: HashMap::new(),
        }
    }
}

impl Default for MorphirHost {
    fn default() -> Self {
        Self::new()
    }
}

// Implement WASI View trait for wasmtime 41.0
impl WasiView for MorphirHost {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

// Implement the Morphir Runtime Host trait
use morphir::ext::runtime::Host;

impl Host for MorphirHost {
    fn log(&mut self, level: LogLevel, msg: String) {
        let level_str = match level {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        };
        tracing::info!(target: "morphir_ext", level = level_str, "{}", msg);
    }

    fn get_env_var(&mut self, name: String) -> Option<EnvValue> {
        self.env_vars.get(&name).cloned()
    }

    fn set_env_var(&mut self, name: String, value: EnvValue) {
        self.env_vars.insert(name, value);
    }
}

// Implement envelope::Host (empty trait for type-only interface)
impl morphir::ext::envelope::Host for MorphirHost {}

/// Runtime for loading and executing Morphir extensions.
pub struct ExtensionRuntime {
    engine: Engine,
    linker: Linker<MorphirHost>,
}

impl ExtensionRuntime {
    /// Create a new extension runtime.
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        
        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);
        
        // Add WASI to linker (using p2 for WASI Preview 2)
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        
        // Add our custom runtime functions to linker
        Extension::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
        
        Ok(Self { engine, linker })
    }
    
    /// Load an extension from a Wasm component file.
    pub fn load_component(&self, path: impl AsRef<Path>) -> Result<Component> {
        let bytes = std::fs::read(path)?;
        Component::new(&self.engine, &bytes)
    }
    
    /// Load an extension from bytes.
    pub fn load_component_from_bytes(&self, bytes: &[u8]) -> Result<Component> {
        Component::new(&self.engine, bytes)
    }
    
    /// Instantiate an extension and return a handle.
    pub fn instantiate(&self, component: &Component) -> Result<ExtensionInstance> {
        let host = MorphirHost::new();
        let mut store = Store::new(&self.engine, host);
        
        let extension = Extension::instantiate(&mut store, component, &self.linker)?;
        
        Ok(ExtensionInstance {
            store,
            extension,
        })
    }
}

impl Default for ExtensionRuntime {
    fn default() -> Self {
        Self::new().expect("Failed to create extension runtime")
    }
}

/// An instantiated extension ready for execution.
pub struct ExtensionInstance {
    store: Store<MorphirHost>,
    extension: Extension,
}

impl ExtensionInstance {
    /// Initialize the extension with startup data.
    /// Returns (initial_model, initial_commands).
    pub fn init(&mut self, init_data: WitEnvelope) -> Result<(WitEnvelope, WitEnvelope)> {
        let program = self.extension.morphir_ext_program();
        program.call_init(&mut self.store, &init_data)
    }
    
    /// Update the extension state with a message.
    /// Returns (new_model, commands).
    pub fn update(&mut self, msg: WitEnvelope, model: WitEnvelope) -> Result<(WitEnvelope, WitEnvelope)> {
        let program = self.extension.morphir_ext_program();
        program.call_update(&mut self.store, &msg, &model)
    }
    
    /// Get active subscriptions based on the model.
    pub fn subscriptions(&mut self, model: WitEnvelope) -> Result<WitEnvelope> {
        let program = self.extension.morphir_ext_program();
        program.call_subscriptions(&mut self.store, &model)
    }
    
    /// Get extension info/help.
    pub fn info(&mut self) -> Result<WitEnvelope> {
        let program = self.extension.morphir_ext_program();
        program.call_info(&mut self.store)
    }
    
    /// Set an environment variable in the host.
    pub fn set_env_var(&mut self, name: String, value: EnvValue) {
        self.store.data_mut().env_vars.insert(name, value);
    }
    
    /// Get an environment variable from the host.
    pub fn get_env_var(&self, name: &str) -> Option<&EnvValue> {
        self.store.data().env_vars.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_runtime_creation() {
        let runtime = ExtensionRuntime::new();
        assert!(runtime.is_ok());
    }
}
