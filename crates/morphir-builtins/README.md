# morphir-builtins

Builtin extensions bundled with Morphir, supporting both native Rust execution and WASM modes.

## Overview

This crate provides extensions that ship with Morphir CLI and can be used in two ways:

1. **Native execution**: Direct Rust implementation (fast, no WASM overhead)
2. **WASM mode**: Compiled to WASM for testing the extension architecture

## Available Builtins

### `migrate`

Transform Morphir IR between different versions (v3/classic ↔ v4).

**Type**: Transform
**Input**: IR (classic or v4) + target version
**Output**: Migrated IR

## Usage

### Native Mode

```rust
use morphir_builtins::migrate::MigrateExtension;
use morphir_builtins::BuiltinExtension;
use morphir_ext_core::Envelope;

let migrate = MigrateExtension::default();

let request = serde_json::json!({
    "ir": { /* ... IR data ... */ },
    "target_version": "v4",
    "expanded": false
});

let input = Envelope::json(&request)?;
let output = migrate.execute_native(&input)?;
let response: MigrateResponse = output.as_json()?;
```

### WASM Mode (with extension runtime)

```rust
use morphir_builtins::migrate::MigrateExtension;
use morphir_ext::{DirectRuntime, ExtismRuntime};

// Get embedded WASM bytes (requires 'wasm' feature)
#[cfg(feature = "wasm")]
{
let wasm_bytes = MigrateExtension::wasm_bytes()
.expect("WASM not available");

let runtime = ExtismRuntime::new(wasm_bytes.to_vec()) ?;
let direct = DirectRuntime::new(Box::new(runtime));

let result = direct.execute("backend_generate", & input_envelope) ?;
}
```

### Discovery via Registry

```rust
use morphir_builtins::registry::BuiltinRegistry;

let registry = BuiltinRegistry::new();

// List all builtins
for info in registry.list() {
println ! ("{}: {} ({})", info.id, info.name, info.extension_type);
}

// Get specific builtin
if let Some(migrate) = registry.get("migrate") {
let result = migrate.execute_native( & input)?;
}
```

## Features

- `default`: Enables all builtins
- `all-builtins`: All available builtins
- `migrate`: Include migrate extension
- `wasm`: Enable WASM bundling (embeds compiled WASM in binary)

## Building for WASM

To compile builtins to WASM:

```bash
cargo build --target wasm32-unknown-unknown --release -p morphir-builtins
```

The WASM output will be in `target/wasm32-unknown-unknown/release/morphir_builtins.wasm`.

## Integration with CLI

The Morphir CLI uses `morphir-builtins` in native mode for best performance:

```rust
use morphir_builtins::migrate::MigrateExtension;

// Direct native call (fast)
let migrate = MigrateExtension::default();
let result = migrate.execute_native(&input)?;
```

## Adding New Builtins

1. Create a new module in `src/` (e.g., `src/typescript_backend/`)
2. Implement `BuiltinExtension` trait
3. Add optional WASM exports in `wasm.rs`
4. Register in `registry.rs`
5. Add feature flag in `Cargo.toml`

Example structure:

```
src/
  my_extension/
    mod.rs        # Native implementation
    wasm.rs       # WASM exports (cfg(target_family = "wasm"))
```

## Architecture

```
┌─────────────────────────────────────┐
│       morphir-builtins              │
│                                     │
│  ┌──────────────────────────────┐  │
│  │  BuiltinExtension trait      │  │
│  │  - execute_native()          │  │
│  │  - info()                    │  │
│  │  - wasm_bytes() [optional]   │  │
│  └──────────────────────────────┘  │
│           ▲          ▲              │
│           │          │              │
│  ┌────────┴───┐  ┌──┴─────────┐   │
│  │  migrate   │  │  ts_backend │   │
│  │  - Native  │  │  - Native   │   │
│  │  - WASM    │  │  - WASM     │   │
│  └────────────┘  └─────────────┘   │
└─────────────────────────────────────┘
         │                │
         ▼                ▼
    Native Mode      WASM Mode
    (CLI direct)     (ExtismRuntime)
```

## License

Apache-2.0
