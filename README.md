[![FINOS - Incubating](https://cdn.jsdelivr.net/gh/finos/contrib-toolbox@master/images/badge-incubating.svg)](https://finosfoundation.atlassian.net/wiki/display/FINOS/Incubating)

# Morphir Rust

Rust-based tooling for the Morphir ecosystem. This project provides a multi-crate workspace including a CLI tool and core libraries for working with Morphir IR (Intermediate Representation).

## Overview

Morphir Rust is part of the Morphir ecosystem, which includes:
- [finos/morphir](https://github.com/finos/morphir) - Core Morphir specification
- [finos/morphir-elm](https://github.com/finos/morphir-elm) - Reference implementation (Elm)
- [finos/morphir-jvm](https://github.com/finos/morphir-jvm) - JVM implementation
- [finos/morphir-scala](https://github.com/finos/morphir-scala) - Scala implementation
- [finos/morphir-dotnet](https://github.com/finos/morphir-dotnet) - .NET implementation

## Project Structure

This is a Rust workspace containing multiple crates:

- **`morphir`** - CLI tool for working with Morphir IR
- **`morphir-ir`** - Core IR model definitions and utilities
- **`morphir-common`** - Shared utilities (remote sources, caching)

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable version recommended)
- Cargo (comes with Rust)

## Installation

### Quick Install (Recommended)

The easiest way to install morphir with automatic version management:

**Linux / macOS:**
```sh
curl -fsSL https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.ps1 | iex
```

The installer sets up a launcher that automatically downloads the correct version when needed.

### Alternative Methods

**Using [mise](https://mise.jdx.dev/):**
```sh
mise install github:finos/morphir-rust@v0.1.0
mise use github:finos/morphir-rust@v0.1.0
```

**Using [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):**
```sh
cargo binstall --git https://github.com/finos/morphir-rust morphir
```

### Building from Source

```sh
git clone https://github.com/finos/morphir-rust.git
cd morphir-rust
cargo install --path crates/morphir
```

### Version Management

```sh
# Use a specific version
morphir +0.1.0 ir migrate --input ./ir.json --output ./v4.json

# Pin version for a project
echo "0.1.0" > .morphir-version

# Or in morphir.toml
# version = "0.1.0"

# Upgrade to latest
morphir self upgrade

# List installed versions
morphir self list
```

## Usage

### Getting Help

```sh
# Show help
morphir --help

# Show help including experimental commands
morphir --help-all
morphir help --full
morphir help --experimental

# Show version
morphir --version
```

### IR Migration

Convert Morphir IR between format versions (Classic V1-V3 ↔ V4):

```sh
# Migrate local file to V4 format
morphir ir migrate --input ./morphir-ir.json --output ./morphir-ir-v4.json

# Migrate from remote URL
morphir ir migrate \
    --input https://lcr-interactive.finos.org/server/morphir-ir.json \
    --output ./lcr-v4.json

# Migrate from GitHub
morphir ir migrate \
    --input github:finos/morphir-examples@main/examples/basic/morphir-ir.json \
    --output ./example-v4.json

# Migrate to Classic format
morphir ir migrate \
    --input ./morphir-ir-v4.json \
    --output ./morphir-ir-classic.json \
    --target-version classic
```

See [IR Migrate Documentation](docs/ir-migrate.md) for full details.

### JSON Schema Generation

Generate JSON Schema for Morphir IR validation:

```sh
# Output to stdout
morphir schema

# Output to file
morphir schema --output ./morphir-ir-schema.json
```

### Tool Management

Manage Morphir tools, distributions, and extensions:

```sh
# Tools
morphir tool install <tool-name> [--version <version>]
morphir tool list
morphir tool update <tool-name> [--version <version>]
morphir tool uninstall <tool-name>

# Distributions
morphir dist install <dist-name> [--version <version>]
morphir dist list
morphir dist update <dist-name>
morphir dist uninstall <dist-name>

# Extensions
morphir extension install <extension-name> [--version <version>]
morphir extension list
morphir extension update <extension-name>
morphir extension uninstall <extension-name>
```

### Experimental Commands

The following commands are experimental and hidden by default. Use `--help-all` to see them:

```sh
# Validate Morphir IR (experimental)
morphir validate --input ./morphir-ir.json

# Generate code (experimental)
morphir generate --target rust --input ./morphir-ir.json --output ./output

# Transform IR (experimental)
morphir transform --input ./morphir-ir.json --output ./transformed.json
```

## Documentation Generation

Generate man pages, markdown documentation, and shell completions:

```sh
# Install usage CLI (required for doc generation)
mise install usage

# Generate all documentation
mise run docs:generate

# Generate specific types
mise run docs:man          # Man pages
mise run docs:markdown     # Markdown docs
mise run docs:completions  # Shell completions
```

### CLI Reference Documentation

The CLI reference docs in `docs/cli/` are auto-generated from `docs/morphir.usage.kdl`:

```sh
# Using mise task (recommended)
mise run docs:cli

# Or manually:
usage generate markdown --file docs/morphir.usage.kdl --multi --out-dir docs/cli/ --url-prefix /cli/
docs/scripts/add-frontmatter.sh
```

**Important**: To add examples to CLI docs, edit the `long_help` field in `morphir.usage.kdl`, not the generated markdown files. For detailed guides, create separate pages in `docs/` (e.g., `docs/ir-migrate.md`).

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

# CLI Documentation
mise run docs:cli          # Regenerate CLI reference docs from KDL spec
mise run docs:generate     # Generate all docs
mise run docs:man          # Man pages only
mise run docs:markdown     # Markdown only
mise run docs:completions  # Shell completions

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
