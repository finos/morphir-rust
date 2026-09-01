---
layout: default
title: Extension System Design
nav_order: 2
parent: For Contributors
---

# Extension System Design

Morphir extensions provide frontends, backends, validators, and transforms
through the Morphir Extension Protocol (MEP). The current host uses JSON-RPC
2.0 envelopes for both supported runtime kinds: `process` and `wasm`.

The extension and Avro release work described here is available for contributor
testing but has not produced a public release.

## Runtime boundary

- Process extensions exchange `Content-Length` framed MEP messages over
  standard input and output. They retain the ambient rights of the user who
  starts Morphir.
- WASM extensions run through Extism. The guest has no direct filesystem or
  network access. It returns artifacts to the host, which validates their
  relative paths and writes them.

Extism is the current WASM engine, not a protocol runtime name. The current
guest ABI does not use WIT or the WebAssembly Component Model.

## Protocol contract

Every extension declares its identity, supported MEP versions, and typed
capabilities during initialization. The host compares the initialized backend
capability with the installed catalog and lock state before generation. That
comparison includes `generate`, targets, and supported Morphir IR versions.

A backend accepts the exact `GenerateRequest { ir, options }` operation and
returns artifacts and diagnostics. Artifact fields are `path`, `content`, and
`binary`.

## Installation

The CLI installs extensions only by ID from a controlled index:

```console
morphir extension install --index <INDEX> <NAME>
morphir extension list
morphir extension update --index <INDEX> <NAME>
morphir extension uninstall <NAME>
```

`<INDEX>` is a directory containing schema-versioned JSONL release histories under
`extensions/` and artifact bytes below the same controlled root. Installation
verifies the artifact digest and records matching catalog and lock state.

### Distribution schema contract

Extension release manifests, extension locks, and extension catalogs are three
separately versioned distribution formats. Their `schemaVersion` field is a
quoted `"major.minor"` JSON string. Each current format writes the strict JSON
string `"schemaVersion": "1.0"`; this version is independent of Morphir IR
versions.

For each format, the reader accepts versions with the current major whose minor
is between that format's minimum and current supported minor, inclusive. It
rejects future minors and versions with any other major. A schema `"1.0"` release
manifest must include `frontend` metadata when it declares the `frontend`
capability and `backend` metadata when it declares the `backend` capability.
Installation persists the matching metadata in the schema `"1.0"` lock and
catalog records used for activation.

The CLI does not discover extension entries in `morphir.toml` and does not
install raw WASM files, archives, URLs, or directories. It has no `--global`
installation mode. `MORPHIR_HOME` selects an isolated state root for local
testing.

## Contributor workflow

The portable protocol types and extension traits in
`morphir-extension-sdk` compile for native and WASM targets. The Extism PDK and
guest exports compile only for `wasm32`. Keep generation logic in native Rust
and use a thin exported guest adapter.

The Avro backend is the complete in-tree example. From the `morphir-rust`
repository root:

```console
cargo test -p morphir-extension-sdk
cargo test -p morphir-avro-extension
cargo build --release -p morphir-avro-extension --target wasm32-unknown-unknown
mise run extension:artifact:avro
```

The artifact task creates a local bundle. It does not create an install index,
Git tag, or public release.

## Further reading

- [Current extension contributor guide](design/extensions/README.md)
- [Morphir Extension Protocol](https://github.com/finos/morphir/blob/main/docs/design/draft/extensions/protocol.md)
- [Distribution and acquisition](https://github.com/finos/morphir/blob/main/docs/design/draft/extensions/distribution-and-acquisition.md)
- [Accepted WASM runtime and Avro proposal](https://github.com/finos/morphir/blob/main/docs/design/proposals/wasm-extension-runtime-and-avro-backend.md)
- [Avro generation guide](https://github.com/finos/morphir/blob/main/docs/generate/avro.md)

The actor-based, multi-protocol documents under
`design/extensions/actor-based/` are preserved as historical design input.
They are excluded from navigation and search and are not an implementation or
installation guide.
