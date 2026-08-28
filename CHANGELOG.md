# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Connected extension-daemon hosting over JSON-RPC HTTP, with an independently launched daemon conformance fixture and coverage for clean shutdown, connection refusal, and request timeout
- Validated MEP typestate sessions that separate untrusted wire initialization from negotiated application state and preserve indeterminate transport outcomes
- Native MEP process hosting with `Content-Length` framed standard streams, explicit working directory and environment, response validation, timeouts, separate stderr capture, and real child-process conformance tests
- MEP 0.1 lifecycle negotiation and reusable conformance tests that build `morphir-wasm-binding` as an independent guest, load it through the native Extism host, invoke real backend generation, verify diagnostics and capability rejection, and complete shutdown
- YAML project, workspace, and global user configuration with XDG, macOS, and Windows path discovery
- Layered configuration loading: built-in defaults, system (`/etc/morphir` or `%PROGRAMDATA%\morphir`), global user, project, workspace member, `.morphir/morphir.user.{toml,yaml}` override, and `MORPHIR_*` environment variables are merged in precedence order
- `morphir_common::config::merge` (`deep_merge`, `merge_all`) implementing the serialization-independent merge rules, and `morphir_common::config::env` for the environment-variable source
- `morphir_devkit::load_effective_config` and `ConfigLoadOptions` for selecting sources explicitly; `ConfigContext` now reports the merged value and the sources that were consulted
- `morphir config path` and `morphir config show` commands (with `--json`) to inspect configuration sources and the effective configuration; `config show` redacts tokens, passwords, secrets, and API keys
- `morphir_common::config::redact` for hiding credentials before a configuration value is displayed, and `morphir_devkit::builtin_defaults` exposing the built-in defaults layer
- Secret references for environment variables, files, direct commands, and native operating-system keyrings, with provenance-aware resolution and protected diagnostic output
- Layout-derived adjacent user overrides for root `morphir.{toml,yaml}` primaries (`morphir.user.{toml,yaml}`), hidden `.morphir/morphir.{toml,yaml}` primaries (`.morphir/morphir.user.{toml,yaml}`), and dot-config `.config/morphir/config.{toml,yaml}` primaries (`.config/morphir/config.user.{toml,yaml}`), including project, workspace, and member configurations
- `MORPHIR_HOME` environment variable relocating the Morphir home directory (default `~/.morphir`, `%USERPROFILE%\.morphir` on Windows), with `morphir_common::home` providing the shared resolution: the tool, distribution, and extension registries, the global log fallback, and the user-home global configuration candidate follow the relocated home, and remote-source and extension caches move under `$MORPHIR_HOME/cache` so sandboxed and hermetic environments never touch the real user directories

### Changed

- Renamed the `morphir-design` crate to `morphir-devkit`; import it as `morphir_devkit` (the public API is unchanged)
- `load_config_context` now merges every configuration layer instead of only the global user and project files
- A `null` overlay value no longer overrides a lower-precedence value; legacy `morphir.json` projects keep global settings intact
- The workspace minimum supported Rust version is now 1.88 because the native keyring integration requires Rust 1.88 or newer

### Deprecated

### Removed

- **Breaking:** the `morphir` CLI crate, its integration tests, the release workflow that published CLI binaries, and the installer and launcher scripts (`scripts/install.*`, `scripts/morphir.*`). The canonical `morphir` CLI is now built, released, and documented from [finos/morphir](https://github.com/finos/morphir), which consumes this workspace's library crates through a git submodule. Install it by following [Installing Morphir](https://github.com/finos/morphir/blob/main/INSTALLING.md); library crates are unaffected

### Fixed

- Native extension hosts no longer link Extism guest PDK imports; the SDK keeps native authoring tests available while compiling guest exports and host imports only for `wasm32`
- `ir.format_version` defaults to 4, and the configuration model, the built-in defaults layer, and the specification now agree on it. Version 3 remains supported and is covered by tests that pin it through the whole merge chain, so a project can stay on 3 with `ir.format_version = 3`.
- Operational environment variables (`MORPHIR_HOME`, `MORPHIR_LOG_DIR`) are no longer interpreted as configuration keys by the `MORPHIR_*` environment source, so `morphir config show` no longer reports a spurious `home` or `log_dir` setting when they are set

### Security

## [0.2.0] - 2026-01-24

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

[Unreleased]: https://github.com/finos/morphir-rust/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/finos/morphir-rust/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/finos/morphir-rust/releases/tag/v0.1.0
