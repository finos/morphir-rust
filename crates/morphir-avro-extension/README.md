# Morphir Avro extension

`morphir-avro-extension` is a portable Morphir Extension Protocol backend for
Apache Avro. It accepts Morphir IR v3 and v4 and returns Avro artifacts through
the typed `GenerateRequest { ir, options }` operation. The crate is implemented
and tested in this repository, but it has not been released.

The native library contains the pure normalization, projection, and rendering
pipeline. The `wasm32-unknown-unknown` build adds a thin MEP guest. The guest
does not read files, write files, or use the network. A Morphir host reads IR,
calls the guest, validates its returned relative artifact paths, and publishes
the files.

## Build and test

From the `morphir-rust` repository root:

```console
cargo test -p morphir-avro-extension
cargo build --release -p morphir-avro-extension --target wasm32-unknown-unknown
mise run test:avro-idl
```

Create the complete local release bundle with:

```console
mise run extension:artifact:avro
```

The bundle appears under `.morphir/build/extensions/avro/`. This command does
not create a tag or publish a release.

## Backend contract

The extension advertises target `avro`, Morphir IR versions `3` and `4`, and the
MEP backend `generate` capability. Generation supports three projections:

- `schemas` emits public types without messages.
- `protocol-entry-points` emits declared v4 Application entry points.
- `protocol-public` emits every public value specification.

JSON output uses `.avsc` for schemas and `.avpr` for protocols. IDL output uses
`.avdl` for every projection. Defaults are JSON, schemas, self-contained
dependencies, inline aliases, strict unsupported-form handling, logical types
enabled, decimal precision 38, and decimal scale 10.

Value bodies never enter the projection model. A public zero-argument value can
become a constant message in `protocol-public`, but the message contains only
`morphir.value-kind = constant` metadata. It never contains or evaluates the
constant value.

## Release tags

An independent extension tag has the form
`extension/avro/v<crate-version>`. A workspace tag has the form
`v<workspace-version>` and includes Avro while its
`.github/extensions.toml` entry has `release_with_workspace = true`. Artifact
creation and release publication are separate jobs. The publication job uses
the exact bundle created by the build job and never rebuilds it.

See
[`docs/contributors/design/extensions/README.md`](../../docs/contributors/design/extensions/README.md)
for the contributor release workflow.
