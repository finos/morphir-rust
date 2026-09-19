# Native Gleam extension

The Gleam extension compiles types to Morphir IR v3 and v4, generates Gleam,
and supports incremental compilation through the Morphir Extension Protocol.
Its extension identifier is `morphir-gleam`. The Rust crate and WASM artifact
remain `morphir-gleam-binding`.

The Morphir CLI includes the extension as a native Rust provider. Compilation
and generation use direct in-process calls by default, without installing an
extension or running a Gleam executable. The same implementation supports MEP
invocation and an optional WASM package.

For a Gleam project, select it explicitly with:

```sh
morphir compile --extension morphir-gleam
morphir generate --target gleam
```

The CLI still accepts `morphir-gleam-binding` as a legacy selector. An installed
extension with that exact old id takes precedence over the alias. Existing
installations retain their recorded identity; newly published bundles use
`morphir-gleam`.

The frontend uses the official [`gleam-core`](https://github.com/gleam-lang/gleam/tree/v1.18.1/compiler-core)
parser, pinned to Gleam 1.18.1 (commit `4a83802ca33a8a96227a1b332768725f232f9779`).
An adapter translates its untyped AST into the extension's lowering model; it
does not run Gleam's type inference or compiler backend. Syntax errors and
incomplete editor-recovery nodes are rejected before IR generation. The Cargo
lockfile retains upstream's `ecow` 0.2.6 because its broad dependency bound also
admits an incompatible newer release.

Version 0.3.0 includes breaking changes to the public Rust API: pretty-printer
entry points now return `Result`, and the AST has additional fields and variants
for unsupported syntax. Rust callers must handle rendering errors and update
AST construction and exhaustive matches when upgrading from 0.2.

## Type compatibility

The frontend resolves aliases, generic custom types, opaque types, labelled
constructors, tuples, function types, module imports, import aliases and
unqualified imported types. It checks type names, visibility, argument counts
and circular aliases. Diagnostics include source locations.

Sum types (discriminated unions) use custom types with multiple constructors.
Each constructor retains its name and its own payload types. Nullary variants,
labelled fields, generic payloads and recursive custom types compile and generate
in both IR versions, for example:

```gleam
pub type Outcome(a) {
  Pending
  Succeeded(value: a)
  Failed(message: String, code: Int)
}
```

Gleam records are ADTs with labelled constructor fields, as described in the
[Gleam language tour](https://tour.gleam.run/data-types/records/). The frontend
preserves their constructors, labels and field types. The backend also represents
a closed Morphir record alias as a single-constructor Gleam ADT. That mapping
preserves fields but changes a structural alias into a nominal custom type;
compiling it back therefore produces a custom type. Open extensible records and
anonymous nested structural records are rejected with a diagnostic.

Built-in type references point to the Morphir SDK. Gleam's `Result(success,
error)` reverses the SDK's error/success parameter order. Imports and module
paths are emitted using Gleam spelling, while IR uses canonical Morphir names.

## IR versions and values

Both `3`/`3.0.0` and `4`/`4.0.0` are accepted. Other releases fail explicitly.
The v3 compiler matches Elm-native's type-only contract. Functions and constants
receive `GLEAM_VALUE_SKIPPED` diagnostics. V4 retains the existing Gleam value
lowering; this work does not establish complete Gleam expression semantics or
native evaluation. Requesting `typesOnly` omits values in either version.
Dependencies may supply v3 or v4 interfaces independently of the output version.

Valid syntax that the adapter cannot lower, such as guarded cases, bit arrays,
external function bindings and record updates, remains available for type-only
compilation and produces an error when compiling values. Labelled function
parameters, labelled calls and imported values also report errors until their
metadata and name resolution can be preserved; labelled type declarations remain
supported. List tails and list patterns preserve their structure, and block
bindings lower to scoped IR destructuring. Operator and value-name resolution
still need work; using the official parser does not establish complete value
semantics. See the [IR coverage inventory](IR_COVERAGE.md) for each value,
pattern and type variant, its tests and remaining limitations.

Constant lowering currently accepts explicitly annotated literals and tuples or
lists of those literals. Function-typed or inferred constants, constant
references, constructor values, list tails and concatenation require further
lowering work and report diagnostics.

## Incremental compilation

The extension is stateless. The caller supplies `CompileBaseline` and receives
per-module results with source and public-interface digests. Unchanged modules
reuse their prior IR. A public-interface change recompiles dependents; changes
to documentation or private declarations do not. A failed module can provide
its last good interface to dependents, while a missing usable interface blocks
them. Independent modules still report their results after a failure. The CLI
persists these results in its workspace cache.

Baselines are scoped to the package, exposure list, output version, semantic
options and dependency interfaces. Malformed entries are discarded with a
warning. The per-module baseline payload is wrapped v4 module IR, including for
a v3 distribution, and is private to this extension's versioned cache context.
Changes to the parser or value-lowering semantics invalidate older baselines.

## Verification and packaging

From the Rust workspace:

```sh
cargo test -p morphir-gleam-binding
cargo build --release -p morphir-gleam-binding --target wasm32-unknown-unknown
MORPHIR_GLEAM_GUEST=target/wasm32-unknown-unknown/release/morphir_gleam_binding.wasm \
  cargo test -p morphir-gleam-binding --features wasm-host-tests --test wasm -- --ignored
```

`mise run extension:artifact:gleam` runs the native suite, builds and validates
the release WASM, compares guest results with native results, packages the
bundle, and tests installation and use without the original package repository.
The bundle is staged under `.morphir/build/extensions/gleam`.

When Gleam is installed, verify generated records, aliases and sum types with:

```sh
MORPHIR_TEST_GLEAM="$(command -v gleam)" cargo test -p morphir-gleam-binding \
  --test backend_types generated_records_aliases_and_opaque_types_pass_the_gleam_compiler -- --ignored
MORPHIR_TEST_GLEAM="$(command -v gleam)" cargo test -p morphir-gleam-binding \
  --test sum_types -- --include-ignored
```

The WASM guest uses in-memory output. Native parse-stage filesystem emission
is unavailable in WASM; set `emitParseStage: false` for identical native/guest
results. Explicit fatal parse-stage requests still report failure.
The official parser also runs inside the WASM guest. Parsing does not need
randomness; the guest explicitly returns an unsupported error if an upstream
code path requests entropy.
