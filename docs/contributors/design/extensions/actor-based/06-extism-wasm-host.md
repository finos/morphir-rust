---
layout: default
title: Extism WASM Extension Host
nav_order: 8
parent: Morphir Extensions
---

# Extism WASM Extension Host

**Status:** Draft  
**Version:** 0.1.0

## Overview

The Extism WASM Extension Host manages extensions compiled to WebAssembly using the Extism framework. This provides strong sandboxing, portability, and support for many languages.

## Protocol

### Runtime

- **Runtime**: Extism (based on Wasmtime)
- **Supported Languages**: Rust, Go, JavaScript/TypeScript, Python, C/C++, Zig, AssemblyScript, and more
- **Sandboxing**: Full WASM isolation with configurable memory/CPU limits

### Interface

Extensions export functions that accept and return JSON strings:

```rust
// Extension exports these functions
fn initialize(config: String) -> String;
fn capabilities() -> String;
fn transform(params: String) -> String;
// ... other methods
```

## Implementation

### State

```rust
use extism::{Manifest, Plugin, Wasm};

pub struct ExtismExtensionHost {
    extensions: HashMap<ExtensionId, ExtismExtension>,
    next_id: u64,
}

struct ExtismExtension {
    name: String,
    plugin: Plugin,
    capabilities: Vec<Capability>,
}
```

### Loading Process

```rust
async fn load_extension(&mut self, config: ExtensionConfig) -> Result<ExtensionId, ExtensionError> {
    let ExtensionSource::Wasm { path } = config.source else {
        return Err(ExtensionError::InvalidSource);
    };

    // Load WASM module
    let manifest = Manifest::new([Wasm::file(&path)]);

    let mut plugin = Plugin::new(&manifest, [], true)?;

    // Initialize
    let config_json = serde_json::to_string(&config.config)?;
    plugin.call("initialize", config_json)?;

    // Get capabilities
    let caps_json = plugin.call("capabilities", "")?;
    let capabilities: Vec<Capability> = serde_json::from_str(&caps_json)?;

    let id = ExtensionId(self.next_id);
    self.next_id += 1;

    self.extensions.insert(id, ExtismExtension {
        name: config.name,
        plugin,
        capabilities,
    });

    Ok(id)
}

async fn call(&mut self, extension_id: ExtensionId, method: &str, params: Value)
    -> Result<Value, ExtensionError> {
    let ext = self.extensions
        .get_mut(&extension_id)
        .ok_or(ExtensionError::NotFound(extension_id))?;

    let params_json = serde_json::to_string(&params)?;

    // Call WASM function (blocking)
    let result_json = tokio::task::spawn_blocking({
        let mut plugin = ext.plugin.clone();
        let method = method.to_string();
        move || plugin.call(&method, params_json)
    }).await??;

    let result: Value = serde_json::from_str(&result_json)?;
    Ok(result)
}
```

## Extension Implementation (Rust + Extism PDK)

```rust
use extism_pdk::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
struct TransformParams {
    ir: Value,
}

#[derive(Serialize)]
struct TransformResult {
    transformed: bool,
    output: Value,
}

#[plugin_fn]
pub fn initialize(config: String) -> FnResult<String> {
    Ok(r#"{"status": "ready"}"#.to_string())
}

#[plugin_fn]
pub fn capabilities() -> FnResult<String> {
    Ok(r#"[{"name": "transform", "description": "Transform IR"}]"#.to_string())
}

#[plugin_fn]
pub fn transform(params_json: String) -> FnResult<String> {
    let params: TransformParams = serde_json::from_str(&params_json)?;

    // Transform IR
    let result = TransformResult {
        transformed: true,
        output: params.ir,
    };

    Ok(serde_json::to_string(&result)?)
}
```

## Extension Implementation (Go + Extism PDK)

```go
package main

import (
    "encoding/json"

    "github.com/extism/go-pdk"
)

//export initialize
func initialize() int32 {
    result := map[string]string{"status": "ready"}
    json, _ := json.Marshal(result)
    mem := pdk.AllocateString(string(json))
    pdk.OutputMemory(mem)
    return 0
}

//export capabilities
func capabilities() int32 {
    caps := []map[string]string{
        {"name": "transform", "description": "Transform IR"},
    }
    json, _ := json.Marshal(caps)
    mem := pdk.AllocateString(string(json))
    pdk.OutputMemory(mem)
    return 0
}

//export transform
func transform() int32 {
    input := pdk.InputString()

    var params map[string]interface{}
    json.Unmarshal([]byte(input), &params)

    result := map[string]interface{}{
        "transformed": true,
        "output": params["ir"],
    }

    output, _ := json.Marshal(result)
    mem := pdk.AllocateString(string(output))
    pdk.OutputMemory(mem)
    return 0
}

func main() {}
```

## Building Extensions

### Rust

```bash
cargo build --target wasm32-unknown-unknown --release
```

### Go

```bash
tinygo build -o extension.wasm -target wasi main.go
```

## Configuration Example

```toml
[[extensions]]
name = "wasm-optimizer"
enabled = true
protocol = "extism"

[extensions.source]
type = "wasm"
path = "./extensions/optimizer.wasm"

[extensions.permissions]
max_memory = "100MB"
max_execution_time = "5s"
```

## Performance Characteristics

- **Latency**: < 1ms per call (after first call)
- **Throughput**: 10,000+ calls/sec
- **Memory**: 1-10MB per extension
- **Startup**: 10-100ms (module compilation)

## Best For

✅ CPU-bound transformations  
✅ Untrusted code (strong sandbox)  
✅ Portable extensions  
✅ High performance  
✅ Near-native speed

❌ Heavy I/O operations  
❌ Extensions needing network access  
❌ Large memory requirements

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
