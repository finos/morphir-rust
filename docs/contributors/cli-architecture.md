---
layout: default
title: CLI Architecture
nav_order: 3
parent: For Contributors
---

# CLI architecture

The Morphir CLI owns command parsing, configuration use, provider registration, and user-facing output. Shared crates provide workspace discovery, extension resolution, verified activation, and protocol sessions.

## Command structure

```text
morphir
├── compile          # Language-neutral compilation
├── generate         # Language-neutral code generation
├── gleam            # Gleam compile, generate, and roundtrip workflows
├── extension        # Verified extension repository and installation management
├── ir               # IR operations
└── ...
```

## Provider registry

The CLI constructs a fresh transport-neutral provider registry, represented by `ExtensionRegistry`, for each operation that needs a provider. It registers the native Gleam extension as a built-in and then adds validated installed-state snapshots. Built-in registration belongs to the CLI because the executable decides which implementations it links. The daemon has no Gleam dependency.

The registry records two independent properties:

- `ProviderOrigin` is `Builtin` or `Installed`.
- `InvocationMode` is `NativeDirect`, `NativeMep`, `ProcessMep`, or `WasmMep`.

Resolution works in this order:

1. Filter providers by the requested frontend or backend operation, language or target, and normalized Morphir IR version.
2. Apply origin precedence among the eligible providers. Installed providers override built-ins.
3. Apply the caller's invocation policy to the selected provider.

`PreferDirect` selects `NativeDirect` for a native built-in. `ProtocolOnly` selects `NativeMep`, which runs the same extension instance through a MEP session. Installed process and WebAssembly providers remain MEP-only under both policies.

This ordering matters. An installed extension that lacks the requested capability or IR version cannot hide an eligible built-in.

Installed records migrated from schemas that did not persist frontend or backend selector metadata remain available for exact-ID activation through the distribution API, but typed registry resolution does not guess their languages, targets, or IR versions. Reinstall the provider from a metadata-bearing index record before relying on automatic capability resolution.

## Compile flow

The general compile path performs these steps:

1. Discover and load the effective project configuration through `morphir-devkit`.
2. Resolve the input and output paths.
3. Read source files into sorted `SourceDocument` values.
4. Construct the CLI-owned provider registry.
5. Resolve `frontend.compile` for the language and Morphir IR version with `PreferDirect`.
6. Invoke the selected native or MEP route.
7. Validate diagnostics and the returned IR, then write `morphir-ir.json` from the host.
8. Format human, JSON, or JSON Lines output.

The single-file Elm compatibility path still launches its configured process extension directly. It does not use the built-in Gleam route.

## Generate flow

Generation performs these steps:

1. Discover configuration and resolve the requested IR input.
2. Prefer `morphir-ir.json` when the input is a compile-output directory. Otherwise load the directory as a Morphir document tree.
3. Detect and normalize the Morphir IR version.
4. Construct the CLI-owned provider registry.
5. Resolve `backend.generate` for the target and IR version with `PreferDirect`.
6. Invoke the selected native or MEP route.
7. Validate returned artifact paths and let the host write the files.
8. Format command output.

Extensions return artifact descriptions. They do not choose arbitrary host filesystem destinations.

## MEP execution

`NativeMep`, `ProcessMep`, and `WasmMep` share the daemon's validated session lifecycle. The host initializes the session, checks provider identity and capabilities, invokes the operation, and shuts the session down. A failed transport preserves whether the peer stopped or entered an indeterminate state.

Installed providers reach that session only after the distribution layer verifies the selected artifact against its catalog and lock state. The registry never treats an unverified file beside the CLI executable as a built-in.

## Output and errors

Human output prints the final success details and diagnostics. JSON pretty-prints one `CompileOutput` or `GenerateOutput`. JSON Lines prints that same result object compactly on one line; it does not emit progress events. The CLI maps configuration, filesystem, provider-resolution, protocol, and extension diagnostics into `miette` reports at the command boundary.

## Further reading

- [Devkit crate](devkit-crate)
- [Extension system design](extension-system)
- [ADR-0002: Validated typestate MEP sessions](adr/0002-validated-typestate-mep-sessions)
- [ADR-0003: Dual native invocation for built-in extensions](adr/0003-dual-native-builtin-invocation)
