---
layout: default
title: WASM Component Model Extension Host
nav_order: 9
parent: Morphir Extensions
---

# WASM Component Model Extension Host

**Status:** Draft  
**Version:** 0.1.0

## Overview

The WASM Component Model Extension Host manages extensions using the Component Model specification. This is the next-generation WASM standard with native support for rich types, interfaces, and composition.

## Protocol

### Runtime

- **Runtime**: Wasmtime with Component Model support
- **Interface Definition**: WIT (WebAssembly Interface Types)
- **Supported Languages**: Rust (best support), C, C++, more coming

### Interface Definition (WIT)

```wit
// morphir-extension.wit
package morphir:extension;

interface extension {
  record capability {
    name: string,
    description: string,
    params-schema: option<string>,
    return-schema: option<string>,
  }

  record init-result {
    status: string,
  }

  initialize: func(config: string) -> init-result;
  capabilities: func() -> list<capability>;
  transform: func(params: string) -> result<string, string>;
}

world morphir-extension {
  export extension;
}
```

## Implementation

### State

```rust
use wasmtime::{
    component::{Component, Linker, ResourceTable},
    Config, Engine, Store,
};
use wasmtime_wasi::WasiView;

pub struct WasmComponentHost {
    engine: Engine,
    linker: Linker<ComponentState>,
    extensions: HashMap<ExtensionId, WasmComponentExtension>,
    next_id: u64,
}

struct WasmComponentExtension {
    name: String,
    instance: MorphirExtension,
    store: Store<ComponentState>,
}

struct ComponentState {
    table: ResourceTable,
    wasi: WasiCtx,
}
```

### Loading Process

```rust
async fn load_extension(&mut self, config: ExtensionConfig)
    -> Result<ExtensionId, ExtensionError> {
    let ExtensionSource::WasmComponent { path } = config.source else {
        return Err(ExtensionError::InvalidSource);
    };

    // Load component
    let component = Component::from_file(&self.engine, &path)?;

    // Create store with WASI
    let mut store = Store::new(
        &self.engine,
        ComponentState {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().build(),
        },
    );

    // Instantiate
    let (instance, _) = MorphirExtension::instantiate_async(
        &mut store,
        &component,
        &self.linker,
    ).await?;

    // Initialize
    instance.call_initialize(&mut store, &config_json).await?;

    // Get capabilities
    let caps = instance.call_capabilities(&mut store).await?;

    let id = ExtensionId(self.next_id);
    self.next_id += 1;

    self.extensions.insert(id, WasmComponentExtension {
        name: config.name,
        instance,
        store,
    });

    Ok(id)
}
```

## Extension Implementation (Rust)

```rust
// Cargo.toml
[dependencies]
wit-bindgen = "0.16"

// lib.rs
wit_bindgen::generate!({
    world: "morphir-extension",
    exports: {
        "morphir:extension/extension": Extension,
    }
});

struct Extension;

impl Guest for Extension {
    fn initialize(config: String) -> InitResult {
        InitResult {
            status: "ready".to_string(),
        }
    }

    fn capabilities() -> Vec<Capability> {
        vec![
            Capability {
                name: "transform".to_string(),
                description: "Transform Morphir IR".to_string(),
                params_schema: None,
                return_schema: None,
            }
        ]
    }

    fn transform(params: String) -> Result<String, String> {
        let input: serde_json::Value = serde_json::from_str(&params)
            .map_err(|e| e.to_string())?;

        // Transform logic
        let output = serde_json::json!({
            "transformed": true,
            "output": input,
        });

        Ok(serde_json::to_string(&output).unwrap())
    }
}
```

## Building

```bash
# Build component
cargo component build --release

# Result: target/wasm32-wasi/release/extension.wasm
```

## Configuration Example

```toml
[[extensions]]
name = "component-backend"
enabled = true
protocol = "component"

[extensions.source]
type = "wasm-component"
path = "./extensions/backend.wasm"

[extensions.permissions]
max_memory = "100MB"
max_execution_time = "10s"
```

## Performance Characteristics

- **Latency**: < 1ms per call
- **Throughput**: 10,000+ calls/sec
- **Memory**: 1-10MB per extension
- **Startup**: 50-200ms

## Best For

✅ Future-proof extensions  
✅ Rich type systems  
✅ Interface composition  
✅ Strong sandboxing  
✅ High performance

❌ Limited language support (currently)  
❌ Requires recent toolchain  
❌ More complex than Extism

## Related

### Morphir Rust Design Documents

- **[Morphir Extensions](../README.md)** - Extension system overview
- **[WASM Components](../wasm-component.md)** - Component model integration
- **[Tasks](../tasks.md)** - Task system definition

### Main Morphir Documentation

- [Morphir Documentation](https://morphir.finos.org) - Main Morphir documentation site
- [Morphir LLMs.txt](https://morphir.finos.org/llms.txt) - Machine-readable documentation index
- [Morphir IR v4 Design](https://morphir.finos.org/docs/design/draft/ir/) - IR v4 design documents
- [Morphir IR Specification](https://morphir.finos.org/docs/morphir-ir-specification/) - Complete IR specification
