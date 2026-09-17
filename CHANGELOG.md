# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Python integer literals preserve arbitrary precision through compilation and
  generation, including hexadecimal, octal and binary source literals.

- Python fixed tuple aliases and tuple-valued expressions now compile and generate,
  including nested tuples and type-checked conditional returns. The binding README
  documents its supported IR nodes, runnable examples and conformance boundaries.

- The Python extension supports annotated pure function bodies with returning
  `if`/`elif`/`else` branches, conditional expressions and scalar comparisons.
  Both compilation and generation validate boolean conditions, branch types
  and return types, with native and WASM roundtrip coverage.

- `morphir-python-binding` provides a Ruff-based Python ADT frontend and backend
  as one native or WASM MEP extension. Frozen dataclasses and named unions map
  to IR v4 records and custom types, with roundtrip tests and explicit diagnostics
  for unsupported source and IR.

- `morphir-package` provides draft `0.1.0-draft.1` metadata normalization,
  exact-byte SHA-256 digests, offline schema validation and closed Library-set
  integrity checks using the Rust IR v4 codec. This experimental package library
  does not resolve dependencies, install packages or write a full `morphir.lock`.
- `mck-adapter-rust --suite package` exposes these operations to the shared MCK
  driver. The default and `--suite ir` retain the existing IR protocol.
- **The YAML profile through the kit.** `morphir_core::ir::yaml` holds the profile's reader and
  its canonical writer (`read`, `write_canonical`, `read_ir_file`, `write_ir_file`), so a YAML
  document becomes the same value tree the JSON reader builds, with the kit's diagnostic codes and
  JSON-pointer cursors. The MCK adapter declares the `yaml` profile.
- `morphir_common::ir_transport::probe_yaml_header` reports the header observations a YAML root
  mapping carries — `format_version_not_first` — as the JSON root probe reports them, so the
  shared conformance corpus is answered for both profiles.
- `morphir-package::resolution` implements the draft `0.1.0-draft.2`
  `flat-library` resolver with phased validation, replay, complete backtracking,
  deterministic update ranking and structured failure witnesses. The MCK adapter
  exposes it with `--suite package --contract 0.1.0-draft.2` while preserving the
  draft-1 package protocol.

### Changed

- **The IR model sweep.** The v4 model now matches the semantic model, the v4 schema and the reference binding where it did not: `DecimalLiteral` is a genuine decimal (`BigDecimal` value beside its lexeme, decimal lexeme grammar), `IntegerLiteral` has arbitrary precision (`BigInt`), type, value and module specifications carry `annotations`, `Hole` incompleteness and `IncompleteBody` keep a `partialBody`, a hole's reason is one of three (`Draft` is an incompleteness), an input type is a bare type, `Documentation` is one string, attribute `constraints` and `extensions` are objects with known members, `$meta` is refused at a single document's root, `priv` is not an access spelling, `inputs` is omitted when empty, and an `Application`'s dependencies are package definitions. The classic (v3) mirror matches morphir-elm: `DecimalLiteral` is a decimal, `DerivedTypeSpecification` exists, record fields are written as `{ "name", "tpe" }` objects, `VariablePattern` is gone, and `Definition` and `ValueDefinition` are one type. The stale second v4 type model (`type_def`), the unused converter and the unused traversal transforms are removed. MCK cases patterns-and-literals-0016 to 0020, types-0012, definitions-0020 to 0031, distributions-0009 and 0010, versions-0006 to 0008.
- **Breaking (`morphir-core` Rust API), from the IR model sweep above.** `Literal::decimal` now
  returns `Result<Literal, InvalidDecimalLexeme>` instead of an infallible `Literal`, and
  `Literal::integer` takes `impl Into<BigInt>` rather than a fixed-width integer.
  `Documentation::new` takes `impl Into<String>` and `Documentation::lines()` returns an iterator
  over the normalized text instead of a stored `Vec`. `Incompleteness::Hole` and
  `ValueBody::Incomplete` are struct variants (`{ reason, partial_body }` and `{ incompleteness,
  partial_body }`) rather than tuple variants. `TypeSpecification`, `ValueSpecification` and
  `ModuleSpecification` each gained an `annotations` field. `InputType` lost its attributes slot
  and is now `InputType(Name, Type)`. MCK definitions-0020 to 0027, 0030.
- The Gleam binding's dependency loader no longer reports `INCOMPATIBLE_DEPENDENCY_DISTRIBUTION`
  for a dependency whose type definition is incomplete: `PackageDefinition::to_specification`
  renders an `IncompleteTypeDefinition` as an ordinary `OpaqueTypeSpecification`, the same shape a
  custom type with private constructors produces, so the loader sees a publishable type rather
  than a distribution it cannot reconcile. The diagnostic is gone because that conversion is now
  infallible.

### Removed

- `morphir_core::ir::v4::{InputTypeEntry, HoleReason::Draft, LegacyTypeSpecification, LegacyTypeDefinition, AccessControlledTypeDefinition, AccessControlledConstructors, TypeDefConstructorArg, TypeDefConstructorDefinition}`; `morphir_core::ir::classic::{Definition, Pattern::Variable}`.
- Input spellings a reader used to accept alongside the canonical ones, as part of the IR model
  sweep above: the bare-string native hint (`"Arithmetic"`, and `"PlatformSpecific"`, which used to
  invent `platform: "unknown"`), the bare-string hole reason `"Draft"`, the Rust-only
  `{ "typeAttributes", "type" }` input-type entry, `priv` as an access spelling, a root `$meta`
  beside `formatVersion`/`distribution`, and an array `doc` outside a module manifest file. Each
  now fails validation instead of being silently accepted. MCK definitions-0024 to 0028, 0031,
  types-0012, distributions-0009.

### Fixed

- **`morphir-common` YAML encoding.** A `DocumentLiteral` number is written with the lexeme it
  was read with. It used to be rounded through `f64` or retyped as a YAML string, which changed
  the payload on a JSON→YAML→JSON round trip; for a while it was refused instead. The canonical
  writer takes the lexeme straight from the value tree, so `9007199254740993` and `0.10` survive
  both profiles and neither encoder can emit serde_json's private number token.
- **`morphir-mck-adapter` syntax probe.** An object whose first member is spelled
  `$serde_json::private::Number` is no longer mistaken for serde_json's internal number token. The
  probe now takes a map for a number only when that is its one member and it holds a string;
  anything else is walked like the object it is, so a duplicate member or a nesting past the
  ceiling inside a document literal that spells a member that way is found rather than skipped.

- **`morphir-daemon` negotiation, `morphir-distribution` publication.** An installed extension no
  longer fails at `initialize` when the display name in its repository record differs from the
  name the guest reports. `repository publish` derives the record name from the identifier
  (`morphir-openapi` became "Morphir Openapi") while the guest reports "Morphir OpenAPI", so every
  published OpenAPI bundle failed with "initialization metadata disagreed with discovery". The
  host now holds only the version and the capability kinds to discovery and names the field that
  drifted. A release bundle's `release.json` may also carry an optional `name`, which
  `.github/extensions.toml` now declares for both extensions and the packaging task emits, so
  future records use the guest's spelling.

### Changed

- **Breaking (`morphir-common` YAML codec, diagnostic codes).** `YamlCodec` decodes and encodes
  through morphir-core: a document is read into the value tree and then into the concrete model,
  and output is the profile's canonical style — block mappings, flow sequences for sequences that
  hold no mapping, plain scalars unless quoting is required, `\n` breaks and exactly one trailing
  newline. Date-looking scalars are strings (`created: 2026-08-28` is the text, not a timestamp),
  and `Literal::Float` carries its lexeme.

  YAML diagnostics now use the kit's codes under `morphir::ir::yaml::`:

  | Was | Is |
  | --- | --- |
  | `morphir::ir::yaml::duplicate_key` | `morphir::ir::yaml::duplicate_member` |
  | `morphir::ir::yaml::alias_not_allowed` | `morphir::ir::yaml::unsupported_yaml_feature` |
  | `morphir::ir::yaml::unsupported_tag` | `morphir::ir::yaml::unsupported_yaml_feature` |
  | `morphir::ir::yaml::merge_key_not_allowed` | `morphir::ir::yaml::unsupported_yaml_feature` |
  | `morphir::ir::yaml::multiple_documents` | `morphir::ir::yaml::invalid_yaml` |
  | `morphir::ir::yaml::non_finite_number` | `morphir::ir::yaml::invalid_literal` |
  | `morphir::ir::yaml::ambiguous_scalar` | removed — the value is a string |
  | `morphir::ir::yaml::invalid_ir` | the specific semantic code |
  | `morphir::ir::yaml::budget_exceeded` | `morphir::ir::yaml::nesting_too_deep`, or the parser's `invalid_yaml` |

  A YAML document with a repeated root `formatVersion` answers `morphir::ir::yaml::duplicate_member`
  rather than `duplicate_format_version`: a repeated member is settled before any member means
  anything. Every other format-version code is unchanged and is the same bare code the JSON path
  answers.
- **Breaking (format-version diagnostics, MCK capabilities).** The diagnostic code
  `unsupported_format_version_revision` is renamed `unsupported_format_version_minor`, with no
  alias: `DiagnosticCode::UnsupportedFormatVersionMinor` in `morphir-core` and
  `NormalizeError::UnsupportedFormatVersionMinor` in `morphir-projection` carry the new spelling,
  and the message reads "release {release} is a minor revision this reader does not support". The
  MCK adapter's capabilities reply gains `formatVersions`, the canonical spelling of this binding's
  support table (`[3.0.0,3.1.0),[4.0.0,4.1.0)`), which `protocol.schema.json` now requires; a
  driver older than that schema refuses the reply as an unknown field.
- Readers accept every patch of a supported minor (`3.0.x`, `4.0.x`);
  `morphir_core::format_version::SupportTable` is now an interval table (`parse`, `canonical`,
  `contains`, `check`, `render_*`), replacing the exact-release list.
- **Breaking (`morphir-core` naming).** `truncate_stem` returns `Option<String>` and answers
  `None` when the budget it is given is below `MIN_TRUNCATED_STEM_BUDGET` (11). A truncated stem is
  `__` plus eight hex digits of the content hash plus at least one character of the name, so a
  smaller budget has no truncation to offer; it used to return the ten-character hash suffix
  anyway, which overran the caller's path budget.
- **Breaking (`morphir-devkit`, `morphir-common` configuration).** One Mill-style out directory
  replaces the per-project output helpers. A workspace has exactly one out root,
  `<workspace>/.morphir/out`; a member never gets its own. Each task owns a scratch directory,
  `<task>.dest`, and a result record beside it, `<task>.json`, and a member's tasks nest under the
  member's path relative to the workspace root, so a member at `packages/orders` compiles into
  `<workspace>/.morphir/out/packages/orders/compile.dest`.

  The new `out` module carries the whole layout: `resolve_out_root` picks the root from the
  `--out-dir` flag, then `MORPHIR_OUT_DIR`, then `[workspace].out_dir`, then the default under the
  workspace root, and is pure, so the caller reads the flag and the environment and passes them in;
  `TaskId` and `TaskPaths` name a task and place it; `TaskResult` and `IrDescriptor` are the record,
  which declares which files are the task's value and where its IR is stored, and preserve fields a
  newer writer added. `ensure_morphir_structure` no longer creates `out/`; the out root creates only
  itself.

  `resolve_compile_output`, `resolve_generate_output`, `resolve_dist_output`, and
  `sanitize_project_name` are removed. Callers move to `TaskPaths`.

  In configuration, `[workspace].output_dir` is renamed to `[workspace].out_dir` and
  `[project].output_directory` is removed. `[ir]` gains `layout` (`single-file` or `document-tree`)
  and `format` (`json` or `yaml`), so IR storage is declared rather than assumed, and `[ir].mode`
  becomes a deprecated optional alias for `layout` that an explicit `layout` overrides. The loader
  warns once for each removed or renamed key it finds and otherwise ignores it, for one release.
  `[workspace].out_dir` set in a member configuration is warned about and ignored: the out root
  belongs to the workspace.
- `morphir-avro-extension` is re-released as `0.1.1`. The extension SDK now states the selected
  target in a generation request, which changed the Avro crate and so the WASM it builds; the
  already-published `extension/avro/v0.1.0` assets no longer match what this commit produces, and
  re-releasing under the same version would have replaced them with different bytes.
- Metadata-only changes can skip the expensive Rust/WebAssembly suite, and the README now
  inventories workspace crates and versioned extensions.
- **Breaking (wire format).** Morphir IR v4 names now encode an initialism as an uppercase
  segment (`value-in-USD`) instead of a run of single letters wrapped in parentheses
  (`value-in-(usd)`), and the parenthesized form is no longer accepted. Existing v4 artifacts
  carrying it will fail to load. The rationale, including the alternatives rejected, is recorded as
  Decision Records 0001 and 0002 in the finos/morphir knowledge base.

  `Name` now holds `Segment::Word` or `Segment::Initialism` values rather than a word list, so a
  backend applies its own convention: Go renders `HTMLParser` where Rust renders `HtmlParser`.
  `Name::from_segments` returns `Result` and validates, `Name::words()` replaces the public `words`
  field, and names project onto a document tree through `to_file_stem`, which is lowercase and
  handles the Windows reserved device names. Decoding accepts the uppercase encoding, the case-free
  doubled-hyphen alternative, and the legacy v1-v3 array; `CANONICAL_STYLE` selects which is written

### Fixed

- Avro extension artifact packaging now preserves `gitCommit` provenance in clean tagged builds by
  preventing Python bytecode caches from making the checkout appear dirty (#130).

### Added

- A reusable extension session. `spawn_session` and `spawn_session_with_idle_timeout` return a
  `SessionHandle` that owns one negotiated MEP session and serves many `invoke` calls from it
  instead of starting a process per call. The session completes the MEP shutdown handshake when
  it is shut down explicitly, when the last handle is dropped, or after an idle period. An
  invocation in flight makes the session ineligible for the idle stop however long it runs, and
  the next idle period starts when it is answered, so the idle setting may be shorter than the
  slowest operation an extension performs. A session that has ended is reported as
  `DaemonError::SessionLost`, which is distinct from an extension refusing one operation, so a
  caller holding a cached handle can tell "evict and respawn" from "retry is pointless";
  `FailedSession::into_error` recovers the underlying cause.
- Verified installation, repair, rollback, and uninstall of explicitly unsigned local developer
  tool packages, with durable provenance and automatic migration of existing tool state.
- A portable `morphir-openapi` WASM backend that projects Morphir v3 and v4 specifications into
  OpenAPI 3.1, OpenAPI 3.0, and JSON Schema 2020-12 documents. One installed extension serves both
  the `openapi` and `json-schema` targets. It renders schemas only, operations synthesized from
  declared entry points, or operations from every public value specification, with per-operation
  method, path, and parameter overrides and optional `Result` response splitting. It can be
  released independently with versioned extension tags.
- Idempotent local extension repository initialization, verified release-bundle publication with
  atomic metadata updates, and repository-qualified search across enabled endpoints.
- Morphir Home-backed named extension repositories with locked lifecycle updates, local-directory
  metadata verification, offline inspection, deterministic resolution, and structured events.
- A versioned, bounded trusted cache-ownership registry with pre-write ownership invalidation,
  atomic producer handoff, unclassified-content protection, guarded inventory/cleanup sessions,
  terminal-entry compaction, structured events, and durable Morphir Home persistence shared by CLI
  and Desktop.
- Durable cache-maintenance scheduling state with interval gating, validated continuation cursors,
  bounded fail-closed loading, and atomic Morphir Home persistence shared by CLI and Desktop.
- Workspace-provider extensions now advertise a typed Morphir workspace discovery capability;
  extension acquisition, negotiation, and daemon result validation enforce that contract.
- Native workspace discovery now exposes a typed error and detailed entry point so hosts can
  distinguish portable discovery failures from native host failures while the existing API keeps
  its compatible error presentation.
- A portable `morphir-avro` WASM backend that projects Morphir v3 and v4 specifications into Avro
  JSON schemas, JSON protocols, or Avro IDL. The backend supports schema-only, entry-point protocol,
  and public protocol modes; represents constants as zero-argument messages with
  `morphir.value-kind=constant`; and can be released independently with versioned extension tags.
- Bounded Extism hosting and installed-extension activation for MEP backends, including exact
  artifact verification, locked backend capability metadata, and configurable generation options.
- Canonical portable workspace discovery for native daemon and browser hosts, with a root-confined
  native adapter, deterministic browser WASM package, and cross-runtime conformance corpus
- A shared, side-effect-free cache maintenance policy planner with portable owned-entry identities,
  active-lease and unclassified protections, deterministic age-then-LRU selection, bounded byte
  accounting, stable decision reasons, and serializable dry-run results.
- A bounded Morphir Home cache inventory that measures registered namespace entries without
  following links or junctions and reports unknown or unsafe content as protected, unclassified
  data.
- A lock-serialized, budgeted cache cleanup executor that revalidates current ownership, leases,
  byte counts, and link safety before moving selected entries through Morphir Home maintenance
  trash.
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
- `morphir-mck-adapter`, an `mck-adapter-rust` binding that drives this workspace's v4 codec through the [Morphir Compatibility Kit](https://github.com/finos/morphir/tree/main/spec/ir/mck), and the `check:kit` task and `kit-conformance` CI job that run the kit against it on every change
- A `Diagnostic` type carrying the kit's stage, code, message, and cursor, and `DocumentLiteral` for a v4 document-tree literal value
- The document-tree file-stem projection now escapes and truncates a name the filesystem cannot hold verbatim, in place of the ad hoc handling it replaced

### Changed

- Renamed the `morphir-design` crate to `morphir-devkit`; import it as `morphir_devkit` (the public API is unchanged)
- `load_config_context` now merges every configuration layer instead of only the global user and project files
- A `null` overlay value no longer overrides a lower-precedence value; legacy `morphir.json` projects keep global settings intact
- During greenfield development, the workspace Rust baseline follows the current stable release and is now Rust 1.98
- **Breaking (Morphir IR v4 JSON vocabulary).** The v4 codec now follows the [Morphir Compatibility
  Kit](https://github.com/finos/morphir/tree/main/spec/ir/mck) and knowledge base Decision Records 0004 through
  0015. A `Record` type or value spells its fields under a `fields` member (0004); every node's optional
  `attributes` member is the first member of its expanded payload (0005); node member names follow the schema
  (0006), with these renames: `attrs` becomes `attributes`; `IfThenElse`'s `thenBranch`/`elseBranch` become
  `then`/`else`; `Field`'s `subject`/`fieldName` become `target`/`name`; `LetDefinition`'s
  `valueName`/`valueDefinition`/`inValue` become `name`/`definition`/`in`. A `Function` type's `arg`/`argumentType`
  becomes `parameterType`, and its `result` becomes `returnType` (0007). A bare array is a `Tuple` at type position
  and a `List` at value position; a bare boolean or number is a literal; a bare string is a `Variable` or a
  `Reference` (0009). The SDK package is canonically spelled `morphir/SDK` (0011). Both v4 schemas share one legacy
  name-array grammar and one `FileStem` definition (0012). Every renamed or restructured spelling above decodes for
  one release with a `legacy_spelling` warning at the member's cursor, and is refused after it.
- **Breaking (public API and emitted bytes), the ripples of the v4 alignment above.**
  `EntryPointKind` gained `Job` and `Policy`, so `x-morphir-entry-point-kind` has two new values a
  consumer may see and a downstream `match` on the enum — public in `morphir-core` and in
  `morphir-projection` — needs two new arms. `morphir-projection` normalizes a
  `DerivedTypeSpecification` to an opaque declaration: a derived type is nominally distinct from
  the type it is built from, and the conversions relating them are not in the model, so a backend
  sees a named type whose structure it must not assume rather than the underlying one. The
  `openapi` and `json-schema` renderers now order every object's member by name at every depth, so
  the bytes they emit change for any document whose builder happened to insert members in another
  order; the projected content is the same. The Gleam backend refuses a dependency whose
  `formatVersion` it does not support (`DEPENDENCY_IR_VERSION_MISMATCH`) instead of carrying on as
  though it were the supported one. Object member order is insertion order workspace-wide:
  `morphir-core` builds `serde_json` with `preserve_order`, and cargo unifies features, so every
  crate in the workspace reading JSON through it keeps a document's member order rather than
  sorting it.

### Deprecated

### Removed

- **Breaking:** the `morphir` CLI crate, its integration tests, the release workflow that published CLI binaries, and the installer and launcher scripts (`scripts/install.*`, `scripts/morphir.*`). The canonical `morphir` CLI is now built, released, and documented from [finos/morphir](https://github.com/finos/morphir), which consumes this workspace's library crates through a git submodule. Install it by following [Installing Morphir](https://github.com/finos/morphir/blob/main/INSTALLING.md); library crates are unaffected
- **Breaking (Morphir IR v4).** The `Native` and `External` value expressions. A native or foreign operation is now
  always a definition body — `NativeBody` or `ExternalBody` — and every use site is a `Reference` to it, per
  knowledge base Decision Record 0008. `ExternalBody` carries a list of per-target bindings and an optional
  fallback body; its single-binding `externalName`/`targetPlatform` spelling decodes for one release as a
  one-entry `externals` list, with a `legacy_spelling` warning, and is refused after it.
- The Classic (IR v3) tagged-array leniency inside a version-4 document. A Classic tagged array read where a v4
  node is expected is now an `unknown_node` refusal through the kit's diagnostic, rather than being accepted as a
  tuple of strings.
- `morphir-common`'s private YAML machinery: `PlainValue` (the number rewrite that stood between a
  value and serde-saphyr), the YAML lexical pre-scan that decided anchors, tags, merge keys and
  timestamps on the source text, the streaming YAML event encoders, and the YAML scanners in the
  root probe (`probe_yaml_slice`, which is no longer exported). The kit's reader and canonical
  writer answer all of it, and `morphir-common` no longer depends on `serde-saphyr`.

### Fixed

- Tool resolution now takes mutable channel membership and active, yanked, or revoked status from
  the current authenticated targets metadata while keeping exact release descriptors immutable.
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
