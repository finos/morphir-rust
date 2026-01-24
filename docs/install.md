---
layout: default
title: Installation
nav_order: 3
---

# Installation

There are several ways to install the Morphir CLI.

## Quick Install (Recommended)

The easiest way to install morphir is using the installer script, which sets up automatic version management.

### Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.sh | bash
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.ps1 | iex
```

The installer:
1. Creates `~/.morphir/bin/` directory
2. Downloads the morphir launcher script
3. Adds the directory to your PATH

After installation, restart your terminal or run:

```bash
source ~/.bashrc  # or ~/.zshrc
```

## Alternative Methods

### Using mise

[mise](https://mise.jdx.dev/) is a polyglot version manager. If you use mise:

```bash
mise install github:finos/morphir-rust@v0.1.0
mise use github:finos/morphir-rust@v0.1.0
```

### Using cargo-binstall

[cargo-binstall](https://github.com/cargo-bins/cargo-binstall) downloads pre-built binaries:

```bash
cargo binstall --git https://github.com/finos/morphir-rust morphir
```

### Manual Download

Download the appropriate binary from the [GitHub Releases](https://github.com/finos/morphir-rust/releases) page.

Available platforms:
- `morphir-{version}-x86_64-apple-darwin.tgz` - macOS Intel
- `morphir-{version}-aarch64-apple-darwin.tgz` - macOS Apple Silicon
- `morphir-{version}-x86_64-unknown-linux-gnu.tgz` - Linux x86_64
- `morphir-{version}-aarch64-unknown-linux-gnu.tgz` - Linux ARM64
- `morphir-{version}-x86_64-pc-windows-msvc.zip` - Windows x86_64

### Build from Source

Requires [Rust](https://www.rust-lang.org/tools/install) (latest stable):

```bash
git clone https://github.com/finos/morphir-rust.git
cd morphir-rust
cargo install --path crates/morphir
```

## Version Management

The morphir launcher automatically manages versions for you.

### Version Resolution

Version is resolved in this order:

1. **Command line override**: `morphir +0.1.0 <command>`
2. **Environment variable**: `MORPHIR_VERSION=0.1.0`
3. **Project file**: `.morphir-version` in current or parent directory
4. **Config file**: `version = "..."` in `morphir.toml`
5. **Latest**: Fetches latest release from GitHub

### Pinning a Version

For reproducible builds, pin the version in your project:

```bash
# Option 1: .morphir-version file
echo "0.1.0" > .morphir-version

# Option 2: morphir.toml
cat > morphir.toml << 'EOF'
version = "0.1.0"
EOF
```

### Self Commands

```bash
# Upgrade to latest version
morphir self upgrade

# List installed versions
morphir self list

# Show which version will be used
morphir self which

# Install a specific version
morphir self install 0.1.0

# Remove old versions
morphir self prune

# Update the launcher script itself
morphir self update

# Show dev mode status
morphir self dev
```

## Dev Mode

Dev mode allows you to run morphir from a local source checkout instead of a downloaded binary. This is useful for:

- Developing and testing changes to morphir
- Debugging issues with local modifications
- CI/CD pipelines that build from source

### Enabling Dev Mode

There are several ways to enable dev mode:

```bash
# One-time: use --dev flag
morphir --dev <command>

# Session: set environment variable
export MORPHIR_DEV=1

# Project: create .morphir-version with "local-dev"
echo "local-dev" > .morphir-version

# Config: add to morphir.toml
cat >> morphir.toml << 'EOF'
[morphir]
dev_mode = true
EOF
```

### Source Directory Detection

When dev mode is enabled, the launcher automatically searches for the morphir-rust source directory in this order:

1. `MORPHIR_DEV_PATH` environment variable
2. CI environment variables (`GITHUB_WORKSPACE`, `CI_PROJECT_DIR`, etc.)
3. Current directory and parent directories
4. Common development locations (`~/code/morphir-rust`, `~/dev/morphir-rust`, etc.)

### Dev Mode Status

Check your dev mode configuration:

```bash
morphir self dev
```

This shows:
- Whether dev mode is enabled and why
- The detected source directory
- Available debug/release binaries

## Environment Variables

| Variable | Description |
|----------|-------------|
| `MORPHIR_VERSION` | Override version to use |
| `MORPHIR_HOME` | Override home directory (default: `~/.morphir`) |
| `MORPHIR_BACKEND` | Force specific backend: `mise`, `binstall`, `github`, `cargo` |
| `MORPHIR_DEV` | Set to `1` or `true` to enable dev mode |
| `MORPHIR_DEV_PATH` | Path to morphir-rust source directory for dev mode |

## Uninstalling

### Launcher Installation

```bash
rm -rf ~/.morphir
# Remove the PATH entry from your shell rc file
```

### Cargo Installation

```bash
cargo uninstall morphir
```

### mise Installation

```bash
mise uninstall github:finos/morphir-rust
```

## Troubleshooting

### Command not found

Ensure `~/.morphir/bin` is in your PATH:

```bash
export PATH="$HOME/.morphir/bin:$PATH"
```

Add this line to your `~/.bashrc` or `~/.zshrc`.

### Permission denied

Make the launcher executable:

```bash
chmod +x ~/.morphir/bin/morphir
```

### Version not downloading

Check your internet connection and try:

```bash
MORPHIR_BACKEND=github morphir self upgrade
```

## Next Steps

- [Getting Started](getting-started) - Basic usage guide
- [CLI Reference](cli/) - Complete command documentation
