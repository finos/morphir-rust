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

## Quick Links

- [GitHub Repository](https://github.com/finos/morphir-rust)
- [FINOS Morphir Project](https://github.com/finos/morphir)
- [LCR Interactive Demo](https://lcr-interactive.finos.org/) - See Morphir in action with the Basel III Liquidity Coverage Ratio regulation

## Getting Started

### Installation

```bash
# Clone and build
git clone https://github.com/finos/morphir-rust.git
cd morphir-rust
cargo build --release

# Or install directly
cargo install --path crates/morphir
```

### Basic Usage

```bash
# Validate a Morphir IR file
morphir validate --input ./morphir-ir.json

# Migrate IR to V4 format
morphir ir migrate --input ./morphir-ir.json --output ./v4.json

# Generate JSON Schema for Morphir IR
morphir schema --output ./morphir-ir-schema.json
```

## Contributing

Morphir Rust is part of the [FINOS](https://www.finos.org/) foundation. Contributions are welcome!

- [Report Issues](https://github.com/finos/morphir-rust/issues)
- [Contributing Guide](https://github.com/finos/morphir-rust/blob/main/CONTRIBUTING.md)
