# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Morphir Home paths for the verified tool store, exact tool locks, and the shared tool-state
  transaction lock.
- A shared verified-file publication boundary for tool and extension content-addressed stores,
  with separate namespaces and common containment, hashing, staging, and reuse checks.
- Strict tool release descriptor types and deterministic stable, preview, insiders, segmented
  preview, and exact-version resolution with CLI compatibility and revocation enforcement.
- TUF-authenticated tool repository loading with bounded root rotation, safe expiration checks,
  descriptor and target metadata cross-checks, and verified artifact downloads.
- Transactional exact tool locks and active catalogs with offline byte re-verification, retained
  rollback releases, failure-safe catalog replacement, and raw executable/AppImage publication.
- Atomic ZIP package staging with traversal, special-file, collision, entry-count, and expanded-size
  defenses plus a durable per-file integrity manifest used by offline activation.
- Safe tar.gz package staging with the same portable-path, collision, special-file, entry-count,
  expansion-size, atomic publication, and offline manifest verification guarantees as ZIP.
- Structured tracing spans and outcome events for tool repository loading, resolution, verified
  downloads, package staging, catalog activation, and offline launch verification.
- Atomic tool rollback to the most recently retained release, including byte re-verification,
  restoration of its original selection, and lock/catalog rollback after write failures.
- Exact-release tool repair that quarantines corrupt or missing active content, rebuilds it from a
  TUF-authenticated download, preserves the installed selection and state, and restores the prior
  bytes if replacement validation fails or the repair process is interrupted.
- Shared `formatVersion` normalization, support-table validation, and replayable JSON/YAML
  root transport probes in `morphir-core` and `morphir-common`, aligned with the parent Morphir
  specification (morphir-l2p9.2)
- The `morphir-okf` and `morphir-kb` crates, backing the `morphir kb` command (#98). `morphir-okf`
  models the Open Knowledge Format v0.2 — bundles, concept documents, frontmatter, markdown link and
  heading extraction, bundle discovery and link resolution — behind an extensible `OkfProfile`.
  `morphir-kb` adds the operations on top: conformance checks, scaffolding, a SQLite FTS5 index,
  upstream sync vendoring, the intent and decision registers, refresh and rendering. Ported from the
  `kb` CLI in morphir-scala, including its JSON shapes and exit codes; the intent register also
  reports `intent-duplicate-id`, which the Scala tool lacks
- Open native JSON/YAML IR codecs, cursor-based semantic events, module-bounded v3-to-v4 migration
  pipelines, and streaming single-file and document-tree transports
- `kb search --index` now applies `--type`, `--tag`, `--status` and `--bundle`, with the same
  semantics the scanning search uses: case-insensitive type and status, every supplied tag required,
  and a bundle matched by label or bare name. They were previously accepted and ignored
- `kb sync diff` gained `--json` and `--raw`. `--json` reports `{path, identical, diff, patch}`;
  `--raw` prints the patch alone, with headers relative to the upstream repository root so it can be
  piped into `git apply`, and prints nothing when the two sides are identical
- `kb sync diff` covers more than one file. Its path argument is now a list, each element either a
  mirrored path or a glob in the dialect `sync.yaml` mappings already use (`*`, `?`, `**`, and
  `**/` matching zero directories); no argument at all means every file the mirror knows about,
  which is the same set `kb sync status` reports on. Only differing files are shown, sorted by
  mirrored path, so `--raw` is a multi-file patch `git apply` takes in one go and `--json` carries
  the per-file records as `{files, summary: {differing, matched}}`. A pattern matching nothing is
  refused by name, and every such pattern is named in one refusal rather than one per run. A
  single mirrored path still renders exactly as it did, in all three forms. A lockfile entry
  absent on both sides is passed over rather than compared, and counted as `absent` in the
  summary instead of being reported as a comparison that never happened
- Atomic, validated installed-extension snapshots that pair each catalog entry with the requested selection from its exact lock
- Transactional extension uninstall that removes active catalog and exact-lock state while retaining verified content-addressed artifact bytes
- `morphir-distribution` verified extension acquisition with strict local JSONL indexes, deterministic channel and exact-version resolution, SHA-256 content-addressed storage, exact locks, an installed catalog, and offline re-verification before process activation
- MEP 0.1 frontend capability negotiation and compile request, result, dependency, diagnostic range, and source document contracts in `morphir-extension-sdk`, including structured extension capabilities and host validation of negotiated compilation support and successful compile results
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
- `MORPHIR_HOME` environment variable relocating the Morphir home directory (default `~/.morphir`, `%USERPROFILE%\.morphir` on Windows), with `morphir_common::home` providing the shared resolution: the tool, distribution, and extension registries, the global log fallback, and the user-home global configuration candidate follow the relocated home. Remote-source and extension caches now default to `<MORPHIR_HOME>/cache` (rather than the platform cache directory), so sandboxed and hermetic environments never touch the real user directories

### Changed

- Renamed the `morphir-design` crate to `morphir-devkit`; import it as `morphir_devkit` (the public API is unchanged)
- `load_config_context` now merges every configuration layer instead of only the global user and project files
- A `null` overlay value no longer overrides a lower-precedence value; legacy `morphir.json` projects keep global settings intact
- During greenfield development, the workspace Rust baseline follows the current stable release and is now Rust 1.98

### Deprecated

### Removed

- **Breaking:** the `morphir` CLI crate, its integration tests, the release workflow that published CLI binaries, and the installer and launcher scripts (`scripts/install.*`, `scripts/morphir.*`). The canonical `morphir` CLI is now built, released, and documented from [finos/morphir](https://github.com/finos/morphir), which consumes this workspace's library crates through a git submodule. Install it by following [Installing Morphir](https://github.com/finos/morphir/blob/main/INSTALLING.md); library crates are unaffected

### Fixed

- Content-addressed artifact paths are now serialized with portable forward slashes on Windows,
  keeping extension installation and offline activation compatible across platforms.
- Native process hosts now complete MEP shutdown by sending the required `morphir.exit`
  notification after the extension acknowledges `morphir.shutdown`
- `kb sync diff` compares an asset as bytes rather than as text. Every mirrored file was decoded as
  UTF-8 first, so an asset holding bytes that are not valid UTF-8 — an image, an archive — came back
  with U+FFFD where those bytes had been and diffed against itself as a change. Concepts are still
  projected as text, which is the only form a frontmatter fence can be removed from. `--raw` now
  emits a real binary patch for such a file, rather than a `Binary files ... differ` line that
  `git apply` refuses — and that would take every other file in a multi-file patch down with it
- Canonical constructor names in v4 custom types now round-trip (#103). An acronym constructor such
  as `GC` serializes to the canonical `"(gc)"`, which the decoders read back as a single literal
  word, leaving a constructor named `(gc)`; the Gleam backend then emitted that into source that
  would not reparse. Decoding goes through `Name::from_canonical_string`, and the backend renders
  identifiers from a name's words rather than its canonical `Display` form
- `kb sync` refuses a mirror `root` that leaves the bundle. A manifest with `root: ../shared` was
  resolved lexically, so `pull` wrote outside the bundle and `pull --prune` deleted files there. An
  absolute root is now rejected outright rather than silently reread as a bundle subdirectory
- `kb sync` quotes lockfile entries that need it. An upstream path containing `,` was silently
  truncated on read — the mirror reported a phantom deleted file while the real one stayed untracked
  — and paths containing `:`, `{` or `}` made `sync.lock.yaml` unparseable. Ordinary paths render
  exactly as before, so a no-op pull still produces no diff
- `kb new-bundle` refuses a `--group` that escapes `kb/bundles`, matching the guard `add-concept`
  already applied
- `kb query` opens the index read-only. The first-token guard admitted every `PRAGMA` and `WITH`
  statement, so `PRAGMA user_version=7` and `WITH … DELETE … RETURNING` could write to the derived
  index through a documented read-only API
- `kb intent` transitions preserve CRLF line endings. Frontmatter normalization meant every
  transition on a CRLF document rewrote the whole file instead of the keys it edits
- `kb sync diff` refuses a path that leaves the mirror. The argument went unchecked, unlike the
  paths `pull` and `push` act on, and a mirror `root` a few directories deep absorbs enough `..`
  segments for the containment check to pass — while the diff's own scratch directory, one level
  above its staging roots, does not. The staging copy and write then landed outside the scratch tree,
  creating directories as they went
- `kb sync diff` no longer calls a file deleted upstream identical. A file edited here and gone from
  the checkout made git fail onto a stderr that was discarded, leaving an empty diff that read as
  "identical" in every output form. Each side is now modelled when it is absent: the file diffs as an
  addition and carries a patch that restores it upstream, and one deleted here diffs as a removal
  instead of dying on an unattributed `No such file or directory`. A path neither side holds is
  refused by name
- `kb search --index --bundle` no longer reaches past the bundle the scan would pick. A bare name
  shared by two bundles — `public/foo` and `private/foo` — matched both, so the indexed search
  returned documents the scanning search excluded, which in a public/private split discloses them

- Classic IR value arguments and parameters now serialize as canonical arrays, matching their strict deserializers and Morphir IR v3 JSON
- Classic IR module definition and specification entries now serialize as canonical two-element arrays, matching their strict deserializers and Morphir IR v3 JSON
- Native extension hosts no longer link Extism guest PDK imports; the SDK keeps native authoring tests available while compiling guest exports and host imports only for `wasm32`
- `ir.format_version` defaults to 4, and the configuration model, the built-in defaults layer, and the specification now agree on it. Version 3 remains supported and is covered by tests that pin it through the whole merge chain, so a project can stay on 3 with `ir.format_version = 3`.
- Operational environment variables (`MORPHIR_HOME`, `MORPHIR_LOG_DIR`) are no longer interpreted as configuration keys by the `MORPHIR_*` environment source, so `morphir config show` no longer reports a spurious `home` or `log_dir` setting when they are set

### Security

- Extension resolution now rejects releases without a host-supported MEP version, and v2 exact locks authenticate launch arguments, capabilities, and MEP versions before activation; legacy v1 locks are rejected explicitly

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
