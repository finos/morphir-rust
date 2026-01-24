---
layout: default
title: Extension Development Tutorial
nav_order: 5
parent: Tutorials
---

# Extension Development Tutorial

Learn how to create Morphir extensions for new languages or targets.

## Overview

Morphir extensions are WASM modules that implement the `Frontend` and/or `Backend` traits. Extensions can be:
- **Builtin**: Bundled with the CLI (like Gleam)
- **Registry**: Installed from a registry
- **Local**: Referenced by path in `morphir.toml`

## Extension Types

### Frontend Extension

Converts source code → Morphir IR V4

```rust
impl Frontend for MyExtension {
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        // Parse source files
        // Convert to Morphir IR V4
        // Return CompileResult
    }
}
```

### Backend Extension

Converts Morphir IR V4 → Generated code

```rust
impl Backend for MyExtension {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        // Read Morphir IR
        // Generate target language code
        // Return GenerateResult
    }
}
```

## Creating a Simple Extension

### Step 1: Create Extension Crate

```bash
cargo new --lib my-morphir-extension
cd my-morphir-extension
```

### Step 2: Configure for WASM

In `Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
morphir-extension-sdk = { path = "../../morphir-extension-sdk" }
extism-pdk = "1.2"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

### Step 3: Implement Extension

In `src/lib.rs`:

```rust
use morphir_extension_sdk::prelude::*;

#[derive(Default)]
pub struct MyExtension;

impl Extension for MyExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "my-extension".into(),
            name: "My Extension".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            types: vec![ExtensionType::Frontend],
            // ...
        }
    }
}

impl Frontend for MyExtension {
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        // Implementation
        Ok(CompileResult {
            success: true,
            ir: vec![],
            diagnostics: vec![],
        })
    }
}
```

### Step 4: Build WASM

```bash
cargo build --target wasm32-unknown-unknown --release
```

### Step 5: Use Extension

Add to `morphir.toml`:

```toml
[extensions.my-extension]
path = "./extensions/my-extension.wasm"
enabled = true
```

## Builtin Extensions

Builtin extensions (like Gleam) are:
- Compiled as part of the CLI build
- Bundled in the CLI binary or resources
- Automatically discovered

To make an extension builtin:
1. Add to `morphir-design/src/extensions.rs`
2. Bundle WASM in build process
3. Update extension discovery

## Extension Discovery

Extensions are discovered in this order:
1. Builtin extensions (highest priority)
2. Extensions from `morphir.toml`
3. Registry extensions

## Extension Configuration

Extensions can have custom configuration:

```toml
[extensions.my-extension]
path = "./extensions/my-extension.wasm"
enabled = true

[extensions.my-extension.config]
custom_setting = "value"
```

Access in extension:

```rust
let custom_setting = request.options
    .get("custom_setting")
    .and_then(|v| v.as_str());
```

## Testing Extensions

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile() {
        let ext = MyExtension;
        let request = CompileRequest {
            sources: vec![],
            options: Default::default(),
        };
        let result = ext.compile(request).unwrap();
        assert!(result.success);
    }
}
```

### Integration Tests

Use the CLI to test extensions:

```bash
morphir compile --language my-language --input src/
```

## Best Practices

1. **Error Handling**: Provide clear diagnostics with source locations
2. **Performance**: Optimize parsing and conversion for large codebases
3. **Testing**: Include comprehensive test coverage
4. **Documentation**: Document language-specific features and limitations

## Next Steps

- Read [Extension System Design](../contributors/extension-system)
- See [Architecture Overview](../contributors/architecture)
- Check [Development Guide](../contributors/development)
