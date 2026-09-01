---
layout: default
title: Extension Development Tutorial
nav_order: 5
parent: Tutorials
---

# Extension development tutorial

This tutorial follows the current Extism and MEP JSON-RPC extension model. The
feature is implemented for contributor testing but has not produced a public
release.

## 1. Start from the reference backend

Use `crates/morphir-avro-extension` as the working example. It separates the
native projection and rendering pipeline from a thin `wasm32` guest adapter.
That boundary keeps backend behavior testable without starting a WASM runtime.

An extension implements `Extension` to declare identity and typed
capabilities. A backend also implements `Backend` and accepts the exact
`GenerateRequest { ir, options }` request. Generated files are returned as
artifacts for the host to validate and write.

## 2. Test the native implementation

Run the SDK conformance tests and the backend's native tests before building a
guest:

```console
cargo test -p morphir-extension-sdk
cargo test -p morphir-avro-extension
```

Keep parsing, normalization, projection, and rendering in native Rust. Test
diagnostics and artifact paths there as ordinary values.

## 3. Build the WASM guest

Build the guest for the current Extism runtime:

```console
cargo build --release -p morphir-avro-extension --target wasm32-unknown-unknown
```

The guest has no direct filesystem or network access. It receives MEP requests
and returns diagnostics and artifacts. The host rejects unsafe or conflicting
paths before writing output.

For the Avro backend, the repository task runs the native tests, builds and
validates the guest, checks IDL goldens, and creates a local bundle:

```console
mise run extension:artifact:avro
```

The task does not publish a release or create an install index.

## 4. Install from a controlled index

Morphir does not discover a local extension path from `morphir.toml`. The CLI
resolves an extension ID from a controlled schema-versioned index, verifies its
digest, and writes matching catalog and lock state.

Each JSONL release manifest uses `"schemaVersion": "1.0"`. If its
`capabilities` include `frontend` or `backend`, the record must contain the
matching `frontend` or `backend` metadata. The resulting extension lock and
catalog are separate formats that also currently write the strict JSON string
`"schemaVersion": "1.0"`. Their readers accept supported minors of the same
major from the minimum through the current minor, reject future minors and
other majors, and do not use this field as a Morphir IR version.

```console
morphir extension install --index <INDEX> <NAME>
morphir extension list
```

The root
[Avro generation guide](https://github.com/finos/morphir/blob/main/docs/generate/avro.md#build-and-install-the-local-extension)
contains a reproducible local index record and the exact commands for an
isolated `MORPHIR_HOME`.

## 5. Exercise the backend

After installation, select a target through the root CLI:

```console
morphir generate --target avro \
  --input morphir-ir.json \
  --output generated/avro
```

Backend-specific defaults live under `[codegen.<target>]` in `morphir.toml`.
Repeat `--option <KEY=VALUE>` for one-command overrides.

## Next steps

- Read the [extension development reference](../extensions/DEVELOPMENT.md).
- Read the [current extension contributor guide](../contributors/design/extensions/README.md).
- Use the [Avro generation guide](https://github.com/finos/morphir/blob/main/docs/generate/avro.md)
  for supported options and output behavior.
