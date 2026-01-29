# morphir-ext-example

Example Morphir extension demonstrating the TEA (The Elm Architecture) pattern with a simple counter.

## Building

This extension is a WebAssembly component meant to be loaded by the Morphir runtime. It must be built for the `wasm32-unknown-unknown` target.

### Build as WebAssembly

```bash
# From the project root
cargo build --package morphir-ext-example --target wasm32-unknown-unknown

# Or from this directory (uses .cargo/config.toml)
cd crates/morphir-ext-example
cargo build
```

The compiled WebAssembly module will be at:
```
target/wasm32-unknown-unknown/debug/morphir_ext_example.wasm
```

### Release build

```bash
cargo build --package morphir-ext-example --target wasm32-unknown-unknown --release
```

## Why WebAssembly?

This crate is configured as a `cdylib` (C dynamic library) to be loaded as a WebAssembly component. It cannot be built for native targets like Linux/macOS/Windows. The WebAssembly component model provides:

- **Sandboxing**: Extensions run in isolated environments
- **Portability**: Same extension works across all platforms
- **Safety**: Memory safety guarantees of WebAssembly
- **Interoperability**: Standard WIT (WebAssembly Interface Types) for communication

## Architecture

The extension implements the TEA pattern:

- **init**: Initialize state with flags
- **update**: Handle messages and update state
- **subscriptions**: Declare event subscriptions
- **get-capabilities**: Return extension metadata

All communication with the host uses the Envelope protocol defined in `morphir-ext-core`.
