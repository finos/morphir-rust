use wasmtime::component::{bindgen, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView, WasiCtxView};
use std::collections::HashMap;

// Generate bindings for wasmtime 41.0
// Note: In wasmtime 41, async is configured per-import, not globally
bindgen!({
    world: "extension",
    path: "../morphir-ext-core/wit",
});

/// Host state for the extension, supporting WASI and Morphir Runtime.
pub struct MorphirHost {
    /// WASI context (file descriptors, environment, etc.)
    ctx: WasiCtx,
    /// Component Resource Table
    table: ResourceTable,
    /// In-memory environment variables (for get/set-env-var)
    env_vars: HashMap<String, morphir::ext::runtime::EnvValue>,
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
use morphir::ext::runtime::{Host, LogLevel, EnvValue};

impl Host for MorphirHost {
    fn log(&mut self, level: LogLevel, msg: String) {
        let level_str = match level {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        };
        println!("[{}] {}", level_str, msg);
    }

    fn get_env_var(&mut self, name: String) -> Option<EnvValue> {
        self.env_vars.get(&name).cloned()
    }

    fn set_env_var(&mut self, name: String, value: EnvValue) {
        self.env_vars.insert(name, value);
    }
}
