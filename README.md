[![FINOS - Incubating](https://cdn.jsdelivr.net/gh/finos/contrib-toolbox@master/images/badge-incubating.svg)](https://finosfoundation.atlassian.net/wiki/display/FINOS/Incubating)

# Morphir Rust

Rust-based tooling for the Morphir ecosystem. This project provides a multi-crate workspace of core libraries for working with Morphir IR (Intermediate Representation). These crates power the canonical `morphir` CLI, which lives in [finos/morphir](https://github.com/finos/morphir).

## Overview

Morphir Rust is part of the Morphir ecosystem, which includes:
- [finos/morphir](https://github.com/finos/morphir) - Core Morphir specification
- [finos/morphir-elm](https://github.com/finos/morphir-elm) - Reference implementation (Elm)
- [finos/morphir-jvm](https://github.com/finos/morphir-jvm) - JVM implementation
- [finos/morphir-scala](https://github.com/finos/morphir-scala) - Scala implementation
- [finos/morphir-dotnet](https://github.com/finos/morphir-dotnet) - .NET implementation

## Workspace crates

This table lists the crates in the Cargo workspace.

| Crate | Description |
| --- | --- |
| `morphir-avro-extension` | Apache Avro backend that projects Morphir v3/v4 specifications to Avro JSON schemas, protocols, and IDL. |
| `morphir-builtins` | Bundled native and WebAssembly extensions, currently including Morphir IR migration. |
| `morphir-common` | Shared IR transport, loading, virtual filesystem, remote, home, cache, and pipeline utilities. |
| `morphir-config` | Portable configuration parsing, merging, and environment rules. |
| `morphir-core` | Morphir IR models, format versions, migration, traversal, and naming. |
| `morphir-daemon` | Workspace, build, IDE, and extension services for long-running Morphir tooling. |
| `morphir-devkit` | Workspace, configuration, and extension discovery APIs for developer tools. |
| `morphir-distribution` | Verified Morphir artifact acquisition, installation, and activation. |
| `morphir-elm-binding` | Tree-sitter based Elm frontend and backend for type declarations in Morphir IR v3 and v4, with incremental compilation, available natively and as a WebAssembly extension. |
| `morphir-ext` | Actor-based extension runtime built on Kameo. |
| `morphir-ext-core` | Core extension ABI and envelope protocol types. |
| `morphir-ext-example` | Example TEA counter WebAssembly component extension. |
| `morphir-extension-sdk` | SDK and MEP contracts for WebAssembly extensions. |
| `morphir-gherkin` | Gherkin document model for `.feature` and `.feature.md` files: spans, navigation, prose, and tag, fence and prose extensions. |
| `morphir-gleam-binding` | Gleam frontend and backend extension integration. |
| `morphir-host` | Portable Morphir extension host: MEP handshake, sessions and channels. |
| `morphir-host-native` | Native channels for the Morphir extension host: processes, Extism and in-process guests. |
| `morphir-kb` | Operational layer for OKF knowledge bundles. |
| `morphir-mck-adapter` | The Morphir Compatibility Kit adapter: `mck-adapter-rust`, a JSON-lines transport for IR and opt-in package operations. |
| `morphir-okf` | Pure OKF model, parsing, and loading support. |
| `morphir-openapi-extension` | OpenAPI and JSON Schema backend that projects Morphir v3/v4 specifications to OpenAPI 3.1, OpenAPI 3.0, and JSON Schema 2020-12 documents. |
| `morphir-package` | Experimental model-package normalization, digests, offline schema validation, and closed Library-set verification. |
| `morphir-projection` | Shared Morphir IR normalization that backend extensions project from. |
| `morphir-python-binding` | Ruff-based frontend and backend for a Morphir IR v4 subset of Python ADTs, tuples and conditional functions, available natively and as a WebAssembly extension. |
| `morphir-runtime` | Reserved runtime crate; currently a minimal scaffold. |
| `morphir-rust-binding` | Syn-based Rust frontend and code generator for types, conditional functions, exhaustive pattern matching, calls and typed lambdas in Morphir IR v3 and v4, available natively and as a WebAssembly extension. |
| `morphir-tests` | Shared acceptance and Cucumber test harness. |
| `morphir-wasm-binding` | Backend extension that generates WebAssembly and WAT from Morphir IR. |
| `morphir-workspace` | Portable workspace discovery protocol and algorithms. |
| `morphir-workspace-wasm` | Browser-facing JSON and WebAssembly adapter for workspace discovery. |

The package MVP adapter has a runnable signed initial-resolve example:
`cargo run -p morphir-mck-adapter --example package_mvp_resolve`. It sends the
exact root and input-only fixture files through the adapter session, then checks
the resulting full-lock digest against the independent frozen golden.
For metadata-only refresh, run
`cargo run -p morphir-mck-adapter --example package_mvp_refresh`. It sends only
the lock, policy and signed metadata, then checks the authenticated receipt.
For a scoped update, run
`cargo run -p morphir-mck-adapter --example package_mvp_update`. It submits an
explicit target and the complete signed local fixture, then checks the new full
lock digest while retaining the old lock.

## Extensions

This table lists independently releasable extensions. Registrations come from
`.github/extensions.toml`, and versions come from package manifests.

| Extension | Package | Version | Description |
| --- | --- | --- | --- |
| `morphir-avro` | `morphir-avro-extension` | `0.2.0` | Generates Avro JSON schemas or protocols and Avro IDL from Morphir specifications. |
| `morphir-elm-native` | `morphir-elm-binding` | `0.2.0` | Compiles Elm type declarations to IR v3 and v4 and generates Elm back; incremental. |
| `morphir-gleam` | `morphir-gleam-binding` | `0.3.0` | Native Gleam frontend and backend using the official Gleam parser; IR v3/v4 types and incremental compilation. |
| `morphir-openapi` | `morphir-openapi-extension` | `0.2.0` | Generates OpenAPI 3.1, OpenAPI 3.0, and JSON Schema 2020-12 documents from Morphir specifications. |
| `morphir-python` | `morphir-python-binding` | `0.4.0` | Compiles and generates Python ADTs, fixed tuples, conditional functions, typed calls and unary lambdas across multiple modules, using IR v3 or v4. |
| `morphir-rust` | `morphir-rust-binding` | `0.3.0` | Compiles and generates Rust types, conditional functions, pattern matches, calls and typed lambdas using IR v3 and v4. |

### Read claims from a built extension

Build an extension bundle with `mise run extension:artifact:<id>`, then read the
guest's capability claims from its staged `.wasm`:

```sh
mise run extension:artifact:avro
mise run extension:claims avro
```

`extension:claims` reads the sole `.wasm` in `.morphir/build/extensions/<id>/`.
It fails if the artifact is missing or ambiguous. To inspect any built guest directly:

```sh
cargo run --locked -p morphir-host-native --bin extension-claims -- path/to/guest.wasm
```

The tool uses the daemon's shared Extism container and sends
`morphir.extension.describe` before initialization. Stdout contains only the
returned claim set as JSON, including `claimsVersion`, `protocolVersions`,
`extension`, `capabilities`, and any `requires` or `critical` members. Validation
uses the SDK claim-set reader; output preserves the guest's JSON members rather
than reconstructing them from native metadata. Missing or failed describe
responses and invalid guests exit non-zero with an error on stderr.

Use these claims for the exact artifact being packaged into a version-2 bundle
descriptor. The tool does not modify the descriptor or packaging pipeline.
The real-guest comparison test is opt-in because it requires a release WASM build:

```sh
cargo build --locked --release -p morphir-avro-extension --target wasm32-unknown-unknown
cargo test --locked -p morphir-host-native --test extension_claims -- --ignored
```

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable version recommended)
- Cargo (comes with Rust)

## The Morphir CLI

The canonical `morphir` CLI is built and released from the
[finos/morphir](https://github.com/finos/morphir) repository, which consumes
this workspace's crates through a git submodule.

- Install: [Installing Morphir](https://github.com/finos/morphir/blob/main/INSTALLING.md)
- Command reference: [CLI Reference](https://morphir.finos.org/docs/cli/)

## Documentation Generation

Generate the release notes page:

```sh
mise run docs:generate
```

### CLI Reference Documentation

The Morphir CLI reference (markdown docs, man page, and shell completions) is generated in the [finos/morphir](https://github.com/finos/morphir) repository with `mise run docs:cli` there, and published at [morphir.finos.org](https://morphir.finos.org/docs/cli/). To add examples to CLI docs, edit the `long_about` help text in the CLI source in that repository.

## Development Setup

```sh
# Install Rust toolchain (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and navigate to the project
git clone https://github.com/finos/morphir-rust.git
cd morphir-rust

# Build the project
cargo build

# Run tests
cargo test

# Format code
cargo fmt

# Check for linting issues
cargo clippy
```

### Mise Tasks

This project uses [mise](https://mise.jdx.dev/) for task automation:

```sh
# Install all dev tools (Rust, Ruby, etc.)
mise install

# List available tasks
mise tasks

# Run checks
mise run check:fmt    # Format check
mise run check:lint   # Lint check

# Documentation
mise run docs:generate     # Generate release notes page

# Jekyll Site (test locally)
mise run docs:serve        # Serve at http://localhost:4000

# Release management
mise run release:check     # Pre-release checks
mise run release:version-bump <version>
mise run release:changelog-validate
```

### Testing Documentation Site Locally

The documentation site uses Jekyll with the Poole theme. To test locally:

```sh
# Install dependencies (Ruby via mise)
mise install

# Serve the site with live reload
mise run docs:serve
```

This starts a local server at http://localhost:4000 with live reload enabled.

## Design Principles

This project follows **Functional Domain Modeling** principles:

- **Immutability**: Data structures are immutable by default
- **Type Safety**: Strong typing throughout the codebase
- **Composability**: Functions and data structures are designed to compose
- **Purity**: Functions are pure where possible, with clear separation of side effects

## Contributing

1. Fork it (<https://github.com/finos/morphir-rust/fork>)
2. Create your feature branch (`git checkout -b feature/fooBar`)
3. Read our [contribution guidelines](.github/CONTRIBUTING.md) and [Community Code of Conduct](https://www.finos.org/code-of-conduct)
4. Commit your changes (`git commit -am 'Add some fooBar'`)
5. Push to the branch (`git push origin feature/fooBar`)
6. Create a new Pull Request

_NOTE:_ Commits and pull requests to FINOS repositories will only be accepted from those contributors with an active, executed Individual Contributor License Agreement (ICLA) with FINOS OR who are covered under an existing and active Corporate Contribution License Agreement (CCLA) executed with FINOS. Commits from individuals not covered under an ICLA or CCLA will be flagged and blocked by the FINOS Clabot tool (or [EasyCLA](https://community.finos.org/docs/governance/Software-Projects/easycla)). Please note that some CCLAs require individuals/employees to be explicitly named on the CCLA.

*Need an ICLA? Unsure if you are covered under an existing CCLA? Email [help@finos.org](mailto:help@finos.org)*


## License

Copyright 2022 FINOS

Distributed under the [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0).

SPDX-License-Identifier: [Apache-2.0](https://spdx.org/licenses/Apache-2.0)
