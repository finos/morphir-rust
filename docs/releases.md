---
layout: default
title: Release Notes
nav_order: 7
---

# Release Notes

All notable changes to Morphir Rust are documented here. This project follows [Semantic Versioning](https://semver.org/).

For the full changelog, see [CHANGELOG.md](https://github.com/finos/morphir-rust/blob/main/CHANGELOG.md) on GitHub.

---

## v0.2.0 (January 24, 2026)


### Added

- **Core CLI Commands**: Promoted `compile` and `generate` from experimental to stable
  - `morphir compile` - Compile source code to Morphir IR using language extensions
  - `morphir generate` - Generate code from Morphir IR using target extensions
- **TUI Pager**: Interactive JSON viewer with syntax highlighting and vim-like navigation
  - Visual mode (`v`, `V`) for selecting text
  - Yank to clipboard (`y`) with WSL, X11, Wayland, and macOS support
  - Word motions (`w`, `b`), line jumps (`g`, `G`), and scroll controls
- **Expanded Format**: `--expanded` flag for `morphir ir migrate` produces verbose V4 output
  - Variables: `{"Variable": {"name": "a"}}` instead of `"a"`
  - References: `{"Reference": {"fqname": "...", "args": [...]}}` instead of array format
- **Launcher Script**: Self-updating launcher with version management (`scripts/morphir.sh`)
  - Supports `.morphir-version` file for per-project version pinning
  - Auto-downloads correct version on first run
  - `morphir self upgrade` to fetch latest version
- **Dev Mode**: Run morphir from local source for development and testing
  - Enable via `--dev` flag, `MORPHIR_DEV=1`, `local-dev` in `.morphir-version`, or `dev_mode=true` in `morphir.toml`
  - `morphir self dev` command to check dev mode status and configuration
  - Auto-detects source directory from CI environments and common locations
- **Gleam Binding**: Roundtrip testing infrastructure for Gleam code
  - Compile Gleam to IR V4, generate back to Gleam, verify equivalence
  - Support for todo/panic expressions in parser

### Fixed

- **VFS Consistency**: `MemoryVfs::exists()` now returns `true` for directories, matching `OsVfs` behavior
- **Compile Path Resolution**: `source_directory` from config is now resolved relative to the config file location, not the current working directory

### Changed

- **V4 Compact Format Improvements**:
  - Reference with args now uses array format: `{"Reference": ["fqname", arg1, ...]}`
  - Type variables are bare name strings in compact mode: `"a"`
  - References without args are bare FQName strings: `"morphir/sdk:int#int"`
- **V4 Canonical Naming**: `Name` type now uses kebab-case by default (e.g., `my-function`)
- **Documentation Site**: Restructured with just-the-docs theme and morphir.finos.org branding

## v0.1.0 (January 23, 2026)


### Added

- Initial release of the Morphir Rust CLI toolchain
- **IR Versioning**: Support for both Classic and V4 Morphir IR formats
- **Remote Source Support**: IR migration can fetch from URLs, GitHub releases, and archives
- **Extension System**: Plugin architecture using Extism with JSON-RPC communication
- **Morphir Daemon**: Background service for workspace management and IDE integration
- **CLI Commands**:
  - `morphir validate` - Validate Morphir IR models
  - `morphir generate` - Generate code from Morphir IR
  - `morphir transform` - Transform Morphir IR
  - `morphir tool` - Manage Morphir tools (install/list/update/uninstall)
  - `morphir dist` - Manage Morphir distributions
  - `morphir extension` - Manage Morphir extensions
  - `morphir ir migrate` - Migrate IR between versions
  - `morphir schema` - Generate JSON Schema for Morphir IR
  - `morphir version` - Print version info (supports `--json` for machine-readable output)
- **Multi-platform Binaries**: Pre-built releases for Linux (x86_64, aarch64, musl), macOS (x86_64, aarch64), and Windows (x86_64, aarch64)
- **cargo-binstall Support**: Install pre-built binaries via `cargo binstall morphir`
- **WASM Bindings**: WebAssembly backend for browser and edge deployments
- **Gleam Binding**: Language binding for Gleam frontend/backend


---

## Upgrading

To upgrade to the latest version:

```bash
# Using the launcher
morphir self upgrade

# Using mise
mise upgrade github:finos/morphir-rust

# Using cargo
cargo install --git https://github.com/finos/morphir-rust morphir --force
```

## Reporting Issues

Found a bug or have a feature request? Please [open an issue](https://github.com/finos/morphir-rust/issues/new) on GitHub.
