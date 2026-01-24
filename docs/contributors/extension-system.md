---
layout: default
title: Extension System Design
nav_order: 2
parent: For Contributors
---

# Extension System Design

This document describes the Morphir extension system, including current implementation and future vision.

## Overview

The Morphir extension system allows adding support for new languages and targets through WASM-based extensions. Extensions implement the `Frontend` and/or `Backend` traits to provide language-specific functionality.

## Current Implementation

### Architecture

The current implementation uses:
- **Extism**: WASM runtime for loading and executing extensions
- **Extension Registry**: Manages loaded extensions
- **Extension Container**: Wraps Extism plugin with Morphir-specific functionality

### Extension Types

Extensions can implement:
- **Frontend**: Converts source code → Morphir IR V4
- **Backend**: Converts Morphir IR V4 → Generated code
- **Transform**: IR-to-IR transformations
- **Validator**: IR validation

### Extension Discovery

Extensions are discovered in this order:
1. **Builtin Extensions**: Bundled with CLI (e.g., Gleam)
2. **Config Extensions**: Specified in `morphir.toml`
3. **Registry Extensions**: Installed from extension registry

### Extension Loading

```rust
// Discover extension
let extension = registry.find_extension_by_language("gleam").await?;

// Call extension method
let result: serde_json::Value = extension
    .call("morphir.frontend.compile", params)
    .await?;
```

## Future Vision: Actor Model

The design documents describe a sophisticated actor-based architecture:

### Extension Manager

Central coordinator as a singleton Kameo actor:
- Manages extension lifecycle
- Routes extension requests
- Coordinates extension hosts

### Extension Hosts

One actor per protocol type:
- **JSON-RPC Host**: JSON-RPC protocol extensions
- **gRPC Host**: gRPC protocol extensions
- **Stdio Host**: Standard I/O extensions
- **Extism WASM Host**: Current WASM extensions
- **WASM Component Host**: Future WASM Component Model extensions

### Benefits of Actor Model

- **Isolation**: Each component is an independent actor
- **Concurrency**: Parallel processing across actors
- **Supervision**: Automatic failure recovery
- **Scalability**: Easy to add new protocol types

## Migration Path

The current implementation is designed to be compatible with the future actor model:

1. **Current**: Direct `ExtensionRegistry` usage
2. **Future**: Extension Manager actor routes requests
3. **Compatibility**: Current API can be wrapped in actor messages

## Extension Development

See [Extension Development Tutorial](../tutorials/extension-tutorial) for how to create extensions.

## Next Steps

- Read [Extension Host Interface](design/extension-host-interface)
- See [Extism WASM Host](design/extism-wasm-host)
- Check [Extension Manager](design/extension-manager)
