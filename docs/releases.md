---
layout: default
title: Release Notes
nav_order: 6
---

# Release Notes

All notable changes to Morphir Rust are documented here. This project follows [Semantic Versioning](https://semver.org/).

For the full changelog, see [CHANGELOG.md](https://github.com/finos/morphir-rust/blob/main/CHANGELOG.md) on GitHub.

---

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
