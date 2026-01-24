---
layout: default
title: Getting Started
nav_order: 2
---

# Getting Started

This guide will help you get up and running with Morphir Rust.

## Prerequisites

- A terminal (bash, zsh, PowerShell)
- Internet connection (for downloading binaries)
- Optional: [Rust](https://www.rust-lang.org/tools/install) if building from source

## Installation

The quickest way to install morphir:

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.sh | bash
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.ps1 | iex
```

For more installation options, see the [Installation Guide](install).

## Your First Command

After installation, verify morphir is working:

```bash
morphir --version
```

## Basic Usage

### Migrating Morphir IR

The most common use case is migrating Morphir IR between format versions. Morphir IR has evolved through several versions:

- **Classic (V1-V3)**: Original format used by morphir-elm
- **V4**: New format with improved structure

Convert a local file to V4 format:

```bash
morphir ir migrate --input ./morphir-ir.json --output ./morphir-ir-v4.json
```

Convert from a remote URL:

```bash
morphir ir migrate \
    --input https://lcr-interactive.finos.org/server/morphir-ir.json \
    --output ./lcr-v4.json
```

Convert from GitHub:

```bash
morphir ir migrate \
    --input github:finos/morphir-examples@main/examples/basic/morphir-ir.json \
    --output ./example-v4.json
```

### Generating JSON Schema

Generate a JSON Schema for validating Morphir IR files:

```bash
# Output to stdout
morphir schema

# Output to file
morphir schema --output ./morphir-ir-schema.json
```

### Getting Help

```bash
# Show help
morphir --help

# Show help including experimental commands
morphir --help-all

# Get help for a specific command
morphir ir migrate --help
```

## Version Management

The morphir launcher supports automatic version management, similar to tools like rustup or nvm.

### Using a Specific Version

Override the version for a single command:

```bash
morphir +0.1.0 ir migrate --input ./ir.json --output ./v4.json
```

### Pinning Version for a Project

Create a `.morphir-version` file in your project root:

```bash
echo "0.1.0" > .morphir-version
```

Or add to your `morphir.toml`:

```toml
version = "0.1.0"
```

### Managing Versions

```bash
# Upgrade to latest version
morphir self upgrade

# List installed versions
morphir self list

# Show which version will be used
morphir self which
```

### Dev Mode

For developing and testing morphir itself, you can run from a local source checkout:

```bash
# One-time dev mode
morphir --dev ir migrate --input ./ir.json

# Check dev mode status
morphir self dev
```

See the [Installation Guide](install#dev-mode) for more details.

## Next Steps

- [Installation Guide](install) - Detailed installation options
- [CLI Reference](cli/) - Complete command documentation
- [IR Migrate](ir-migrate) - Detailed migration documentation
