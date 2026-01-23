---
layout: default
title: Home
---

# Morphir Rust Documentation

Welcome to the documentation for **Morphir Rust** - Rust tooling for the Morphir ecosystem.

## What is Morphir?

[Morphir](https://github.com/finos/morphir) is a library of tools that work together to solve different use cases. The central idea behind Morphir is that you write your business logic once as a set of Morphir expressions and then consume them in various ways, including:

- Visualizations
- Code generation
- Type checking
- Optimization
- Execution

## CLI Commands

### Stable Commands

| Command | Description |
|---------|-------------|
| [`ir migrate`](ir-migrate.md) | Convert Morphir IR between format versions |
| `schema` | Generate JSON Schema for Morphir IR |
| `tool` | Manage Morphir tools |
| `dist` | Manage Morphir distributions |
| `extension` | Manage Morphir extensions |

### [IR Migrate](ir-migrate.md)

Convert Morphir IR between format versions (Classic V1-V3 ↔ V4). Supports local files and remote sources.

```bash
# Migrate local file to V4
morphir ir migrate --input ./morphir-ir.json --output ./v4.json

# Migrate from remote URL (e.g., the LCR regulatory model)
morphir ir migrate \
    --input https://lcr-interactive.finos.org/server/morphir-ir.json \
    --output ./lcr-v4.json
```

### Experimental Commands

Use `morphir --help-all` to see experimental commands:

| Command | Description |
|---------|-------------|
| `validate` | Validate Morphir IR models |
| `generate` | Generate code from Morphir IR |
| `transform` | Transform Morphir IR |

## Quick Links

- [GitHub Repository](https://github.com/finos/morphir-rust)
- [FINOS Morphir Project](https://github.com/finos/morphir)
- [LCR Interactive Demo](https://lcr-interactive.finos.org/) - See Morphir in action with the Basel III Liquidity Coverage Ratio regulation

## Getting Started

### Installation

#### Pre-built Binaries (Recommended)

Pre-built binaries are available for Linux (x86_64, aarch64, musl), macOS (x86_64, aarch64), and Windows (x86_64, aarch64).

**Using [mise](https://mise.jdx.dev/):**

```bash
mise install github:finos/morphir-rust@v0.1.0
mise use github:finos/morphir-rust@v0.1.0
```

**Using [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):**

```bash
cargo binstall --git https://github.com/finos/morphir-rust morphir
```

**Manual Download:**

Download the appropriate binary from the [GitHub Releases](https://github.com/finos/morphir-rust/releases) page.

#### Build from Source

```bash
git clone https://github.com/finos/morphir-rust.git
cd morphir-rust
cargo install --path crates/morphir
```

### Getting Help

```bash
# Show help
morphir --help

# Show help including experimental commands
morphir --help-all
morphir help --full
morphir help --experimental
```

### Basic Usage

```bash
# Migrate IR to V4 format
morphir ir migrate --input ./morphir-ir.json --output ./v4.json

# Generate JSON Schema for Morphir IR
morphir schema --output ./morphir-ir-schema.json

# Validate a Morphir IR file (experimental)
morphir validate --input ./morphir-ir.json
```

## Contributing

Morphir Rust is part of the [FINOS](https://www.finos.org/) foundation. Contributions are welcome!

- [Report Issues](https://github.com/finos/morphir-rust/issues)
- [Contributing Guide](https://github.com/finos/morphir-rust/blob/main/CONTRIBUTING.md)
