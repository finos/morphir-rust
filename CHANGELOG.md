# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.1.1] - 2026-01-24

### Added

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

### Changed

- **V4 Compact Format Improvements**:
  - Reference with args now uses array format: `{"Reference": ["fqname", arg1, ...]}`
  - Type variables are bare name strings in compact mode: `"a"`
  - References without args are bare FQName strings: `"morphir/sdk:int#int"`
- **V4 Canonical Naming**: `Name` type now uses kebab-case by default (e.g., `my-function`)
- **Documentation Site**: Restructured with just-the-docs theme and morphir.finos.org branding

## [0.1.0] - 2026-01-23

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

[Unreleased]: https://github.com/finos/morphir-rust/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/finos/morphir-rust/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/finos/morphir-rust/releases/tag/v0.1.0
