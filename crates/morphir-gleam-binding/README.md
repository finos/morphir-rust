# Gleam extension

The Gleam extension compiles types to Morphir IR v3 and v4, generates Gleam,
and supports incremental compilation through the Morphir Extension Protocol.
Its identifier remains `morphir-gleam-binding` for existing configurations.

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
