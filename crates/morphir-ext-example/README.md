# morphir-ext-example

Example Morphir extension demonstrating the TEA (The Elm Architecture) pattern with a simple counter.

## Building

This extension is a WebAssembly component meant to be loaded by the Morphir runtime. It must be built for the `wasm32-unknown-unknown` target.

### Build as WebAssembly

```bash
# From the project root (installs the target if needed, stages the artifact)
mise run build:wasm            # debug
mise run build:wasm release    # release
```

`mise run build:wasm` runs `wasm-tools component new` over cargo's output and stages the resulting **component** at `.morphir/build/ext/morphir_ext_example.wasm`. Cargo alone emits a core module carrying component metadata, which the runtime cannot load. The unconverted core module stays where cargo puts it:

```
target/wasm32-unknown-unknown/debug/morphir_ext_example.wasm
```

To drive the steps directly instead:

```bash
rustup target add wasm32-unknown-unknown

# From the project root
cargo build --package morphir-ext-example --target wasm32-unknown-unknown
wasm-tools component new \
    target/wasm32-unknown-unknown/debug/morphir_ext_example.wasm \
    -o morphir_ext_example.wasm

# Or from this directory (uses .cargo/config.toml)
cd crates/morphir-ext-example
cargo build
```

### Linting

`mise run check:lint:rust` does not cover this crate — it has no native build, so clippy never sees it on the host. Lint it against the wasm target instead:

```bash
mise run check:lint:wasm
```

CI runs both of these in the `Build (WASM extension)` job.

## Why WebAssembly?

This crate is configured as a `cdylib` (C dynamic library) to be loaded as a WebAssembly component. It has no meaningful native build: `wit_bindgen::export!` emits canonical-ABI symbols whose names contain `:`, `/`, `@` and `#`, which are legal wasm export names but invalid identifiers in the ELF version script rustc hands the linker for a `cdylib`. Linking one natively fails with `syntax error in VERSION script`.

So `src/lib.rs` is gated on `#![cfg(target_arch = "wasm32")]`. On a native host the crate compiles to an empty library, which keeps workspace-wide commands (`cargo build --workspace`, `cargo clippy --workspace`) working; on `wasm32-*` it compiles to the real extension.

The WebAssembly component model provides:

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
