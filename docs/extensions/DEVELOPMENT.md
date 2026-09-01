---
layout: default
title: Extension Development Reference
parent: For Contributors
nav_order: 7
---

# Morphir extension development reference

Morphir extensions provide frontends, backends, validators, and transforms
through MEP JSON-RPC. The supported runtime kinds are `process` and `wasm`.
WASM guests run through Extism and do not use WIT or the WebAssembly Component
Model.

The host bounds each WASM guest to 256 MiB of linear memory, a 30-second and
100-million-instruction call budget, and 64 MiB MEP requests and responses.
Discovery and invocation are isolated from Tokio runtime workers.

The extension and Avro release work is available for contributor testing but
has not produced a public release.

## SDK boundary

Implement `Extension` to report a stable identity and typed capabilities. A
backend also implements `Backend` and receives the exact
`GenerateRequest { ir, options }` operation. It returns diagnostics and
artifacts with `path`, `content`, and `binary` fields.

Portable protocol types and extension traits compile for native and WASM
targets. The Extism PDK, guest exports, and imported host functions compile
only for `wasm32`. Keep domain logic in native Rust and use a thin guest adapter
for the MEP dispatcher.

The in-tree `morphir-avro-extension` crate is the reference implementation.

## Build and test

From the repository root:

```console
cargo test -p morphir-extension-sdk
cargo test -p morphir-avro-extension
cargo build --release -p morphir-avro-extension --target wasm32-unknown-unknown
```

To create and validate the local Avro release bundle:

```console
mise run extension:artifact:avro
```

This task creates a local bundle only. It does not create an index, tag, or
public release.

## Install for local testing

The CLI installs an extension ID from a controlled schema-versioned index. It does not
install a raw WASM file, archive, URL, or directory.

```console
morphir extension install --index <INDEX> <NAME>
morphir extension list
```

Use `MORPHIR_HOME` when a test needs isolated catalog, lock, and store state.
The CLI has no `--global` install mode.

See the root repository's
[Avro generation guide](https://github.com/finos/morphir/blob/main/docs/generate/avro.md#build-and-install-the-local-extension)
for a tested build, local index, install, and generation sequence.

## Current documents

- [Morphir extension contributor guide](../contributors/design/extensions/README.md)
- [Extension system overview](../contributors/extension-system.md)
- [Morphir Extension Protocol](https://github.com/finos/morphir/blob/main/docs/design/draft/extensions/protocol.md)
- [Distribution and acquisition](https://github.com/finos/morphir/blob/main/docs/design/draft/extensions/distribution-and-acquisition.md)
- [Accepted WASM runtime and Avro proposal](https://github.com/finos/morphir/blob/main/docs/design/proposals/wasm-extension-runtime-and-avro-backend.md)
