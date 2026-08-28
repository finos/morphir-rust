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

## Project Structure

This is a Rust workspace containing multiple crates:

- **`morphir-ir`** - Core IR model definitions and utilities
- **`morphir-common`** - Shared utilities (remote sources, caching)

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
