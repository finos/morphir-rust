---
layout: default
title: Morphir Extensions
nav_order: 1
parent: Design Documents
has_children: true
---

# Morphir Extensions

Morphir extensions provide frontends, backends, validators, and transforms
through the Morphir Extension Protocol (MEP). The current implementation uses
JSON-RPC 2.0 request and response envelopes for every runtime.

The extension and Avro release work described here is implemented for
contributor testing but has not produced a public release.

## Current architecture

The public runtime names are `process` and `wasm`:

- A process extension exchanges `Content-Length` framed MEP messages over
  standard input and output. It has the ambient rights of the user who starts
  Morphir.
- A WASM extension runs through the Extism engine. The guest has no direct
  filesystem or network access. It returns artifacts to the host, which
  validates and writes them.

The WASM host limits linear memory to 256 MiB, each call to 30 seconds and 100
million instructions, and each MEP request or response to 64 MiB. Discovery and
invocation run on blocking workers so guest execution does not occupy a Tokio
runtime worker.

Extism is an engine detail, not a runtime value or a second protocol. The
current guest ABI is not the WebAssembly Component Model and does not use WIT
interfaces. Files in this directory that discuss WIT components, Wasmtime,
actor-based hosting, raw drop-in discovery, or `.morphir-ext.tgz` packages are
historical design exploration, not instructions for the current CLI.

The portable protocol types and extension traits in
`morphir-extension-sdk` compile for native and WASM targets. The Extism PDK,
guest exports, and imported host functions compile only for `wasm32`. Native
hosts use the Extism runtime adapter and do not link guest PDK imports.

## Implement an extension

Every extension implements `Extension` to declare identity and typed
capabilities. A backend also implements `Backend` and accepts the exact
`GenerateRequest { ir, options }` operation. The SDK export macro adds the
WASM guest entry points and MEP dispatcher.

The [Avro backend implementation](../../../../crates/morphir-avro-extension/src/lib.rs)
is the complete in-tree example. Its shape is:

```text
Extension::info() -> stable identity and version
Extension::capabilities() -> backend { targets, irVersions, generate }
Backend::generate(GenerateRequest { ir, options }) -> GenerateResult
export_extension!(extension type, backend)
```

Keep normalization, projection, and rendering in ordinary native Rust. The
exported guest should be a thin adapter so the domain pipeline can be tested
without a WASM runtime.

## Build and test

From the `morphir-rust` repository root, run focused native tests before
building a guest:

```console
cargo test -p morphir-extension-sdk
cargo test -p morphir-avro-extension
cargo build --release -p morphir-avro-extension --target wasm32-unknown-unknown
```

The SDK conformance fixtures exercise initialization, capability negotiation,
MEP dispatch, generation, cancellation, and shutdown. Runtime tests should run
the same session contract through the process and WASM adapters.

## Install from a controlled index

The CLI does not install a raw `.wasm` file, archive, URL, or directory. It
resolves an extension ID from a controlled local index, verifies the selected
artifact, and writes matching catalog and lock state:

```console
morphir extension install --index <INDEX> <NAME>
morphir extension list
morphir extension update --index <INDEX> <NAME>
morphir extension uninstall <NAME>
```

`<INDEX>` is a directory with JSONL release histories under `extensions/` and
artifact bytes below that controlled root. Installation defaults to the
`stable` channel. Use `--channel <CHANNEL>` or `--version <VERSION>` to select
another release. The CLI has no `--global` mode; `MORPHIR_HOME` selects the
state root when a contributor needs an isolated test home.

Release manifests use the strict JSON string `"schemaVersion": "1.0"` and
must include matching metadata for every declared frontend or backend
capability. Installation writes separate extension lock and catalog formats,
each with its own current `"1.0"` schema version. Each reader accepts supported
minors of the same major from its minimum through its current minor and rejects
future minors and other majors. These distribution versions are independent of
Morphir IR versions.

For a tested Avro build-to-index example, see the root repository's
[Avro generation guide](https://github.com/finos/morphir/blob/main/docs/generate/avro.md#build-and-install-the-local-extension).

## Avro WASM artifacts

Create and validate a local Avro release bundle with:

```console
mise run extension:artifact:avro
```

The task tests the native crate, builds and validates the WASM guest, checks the
Avro IDL goldens, and writes the WASM file, checksum, and `release.json` under
`.morphir/build/extensions/avro/`. It does not create an install index, Git tag,
or GitHub release. The root Avro guide shows how to turn that local bundle into
a schema `"1.0"` local index record.

## Create versus publish

Artifact creation, Git tag creation, and release publication are separate
operations:

- `mise run extension:artifact:avro` creates a local bundle only.
- `git tag ...` creates a local Git reference only.
- Pushing an eligible tag or manually dispatching the release workflow starts
  publication.

The workflow checks out the exact tag, builds selected bundles in a read-only
job, and passes those bytes to a separate publication job. The publisher
verifies the descriptor and checksums and uploads the same bytes. It does not
rebuild an extension.

## Release tags

An independent extension release uses the Avro crate version:

```console
git tag extension/avro/v0.1.0
```

A workspace release uses the workspace version:

```console
git tag v0.2.0
```

`.github/extensions.toml` controls workspace participation. Each entry must set
`release_with_workspace` to a Boolean. When it is `true`, a
`v<workspace-version>` tag includes that extension. When it is `false`,
workspace tags skip it. Either setting still permits an independent
`extension/<short-id>/v<crate-version>` tag. A tag version that differs from
the authoritative Cargo version fails before the build starts.

Do not push a tag or dispatch the workflow merely to test artifact creation.
The Avro extension has not been publicly released.
