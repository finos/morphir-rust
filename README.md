[![FINOS - Incubating](https://cdn.jsdelivr.net/gh/finos/contrib-toolbox@master/images/badge-incubating.svg)](https://finosfoundation.atlassian.net/wiki/display/FINOS/Incubating)

# Morphir Rust

Rust-based tooling for the Morphir ecosystem. This project provides a multi-crate workspace including a CLI tool and core model definitions for working with Morphir IR (Intermediate Representation).

## Overview

Morphir Rust is part of the Morphir ecosystem, which includes:
- [finos/morphir](https://github.com/finos/morphir) - Core Morphir specification
- [finos/morphir-elm](https://github.com/finos/morphir-elm) - Reference implementation (Elm)
- [finos/morphir-jvm](https://github.com/finos/morphir-jvm) - JVM implementation
- [finos/morphir-scala](https://github.com/finos/morphir-scala) - Scala implementation
- [finos/morphir-dotnet](https://github.com/finos/morphir-dotnet) - .NET implementation
- [finos/morphir-go](https://github.com/finos/morphir-go) - Go implementation (coming soon)

## Project Structure

This is a Rust workspace containing multiple crates:

- **`morphir`** - CLI tool for working with Morphir IR
- **`morphir-models`** - Core IR model definitions and utilities

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable version recommended)
- Cargo (comes with Rust)

## Installation

### Building from Source

```sh
# Clone the repository
git clone https://github.com/finos/morphir-rust.git
cd morphir-rust

# Build the project
cargo build --release

# Install the CLI tool
cargo install --path morphir
```

## Usage

### CLI Commands

```sh
# Validate Morphir IR
morphir validate --input path/to/ir

# Generate code from Morphir IR
morphir generate --target rust --input path/to/ir --output path/to/output

# Transform Morphir IR
morphir transform --input path/to/ir --output path/to/output

# Manage Morphir tools
morphir tool install <tool-name> [--version <version>]
morphir tool list
morphir tool update <tool-name> [--version <version>]
morphir tool uninstall <tool-name>

# Manage Morphir distributions
morphir dist install <dist-name> [--version <version>]
morphir dist list
morphir dist update <dist-name> [--version <version>]
morphir dist uninstall <dist-name>

# Manage Morphir extensions
morphir extension install <extension-name> [--version <version>]
morphir extension list
morphir extension update <extension-name> [--version <version>]
morphir extension uninstall <extension-name>
```

### Tool Management

The Morphir CLI provides built-in support for managing Morphir tools, distributions, and extensions:

#### Tools

```sh
# Install a Morphir tool
morphir tool install morphir-scala --version 1.0.0

# List all installed tools
morphir tool list

# Update a tool to a specific version or latest
morphir tool update morphir-scala --version 2.0.0

# Uninstall a tool
morphir tool uninstall morphir-scala
```

Tools are stored in `~/.morphir/tools.json` and can be managed independently of the main CLI installation.

#### Distributions

```sh
# Install a Morphir distribution
morphir dist install morphir-jvm-dist --version 2.0.0

# List all installed distributions
morphir dist list

# Update a distribution
morphir dist update morphir-jvm-dist --version 3.0.0

# Uninstall a distribution
morphir dist uninstall morphir-jvm-dist
```

Distributions are stored in `~/.morphir/distributions.json`.

#### Extensions

```sh
# Install a Morphir extension
morphir extension install scala-backend --version 1.5.0

# List all installed extensions
morphir extension list

# Update an extension
morphir extension update scala-backend --version 2.0.0

# Uninstall an extension
morphir extension uninstall scala-backend
```

Extensions are stored in `~/.morphir/extensions.json`.

### Using the Library

Add to your `Cargo.toml`:

```toml
[dependencies]
morphir-models = { path = "../morphir-models" }
```

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

# Run the CLI
cargo run --bin morphir -- --help

# Format code
cargo fmt

# Check for linting issues
cargo clippy
```

## Design Principles

This project follows **Functional Domain Modeling** principles:

- **Immutability**: Data structures are immutable by default
- **Type Safety**: Strong typing throughout the codebase
- **Composability**: Functions and data structures are designed to compose
- **Purity**: Functions are pure where possible, with clear separation of side effects

## Roadmap

List the roadmap steps; alternatively link the Confluence Wiki page where the project roadmap is published.

1. Item 1
2. Item 2
3. ....

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
