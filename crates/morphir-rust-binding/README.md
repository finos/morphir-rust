# Rust type frontend and backend

`morphir-rust-binding` implements the `morphir-rust` extension. It provides MEP
`frontend/compile` and `backend/generate` natively and as an Extism WASM guest.
Both capabilities advertise IR versions `3` and `4`, language/target `rust` and
source suffix `.rs`. Syn parses source without running rustc, macros or build
scripts. The shared `morphir-core` codecs and typed migration handle the IR.

This first increment translates types. It does not compile function bodies or
implement Morphir value semantics. It does not claim complete Rust language
support or extension-level MCK conformance.

## Source subset

```rust
/// A domain model.
pub struct Person {
    pub name: String,
    pub age: i64,
}

pub enum Decision<T> {
    Pending,
    Accepted(T),
    Rejected { reason: String },
}

pub type Pair<T> = (T, i64);
pub struct AccountId(pub String);
pub struct Marker;
pub struct Node { pub next: Option<Box<Node>> }
```

A compile request takes exactly one source document. Its filename determines
the module, so `models.rs` produces `models`. Use a canonical package name such
as `acme/example` and `exposedModules: ["Models"]` to expose that module. An empty
exposure list keeps it private. Sources are supplied as text; their URIs are
identifiers and are never opened by the extension.

| Rust input | Morphir representation |
| --- | --- |
| Public named struct with public fields | Record alias |
| Recursive named struct | Nominal custom type with one constructor |
| Tuple or unit struct | Nominal custom type with one constructor |
| Struct with private fields | Custom type with private constructor |
| Enum | Custom type; variants become constructors |
| Named enum payload | Constructor argument containing a record |
| Tuple, `()` | Tuple, Unit |
| `type Alias<T> = ...` | Transparent type alias |
| Unconstrained type parameter | Type variable |
| Local type reference | Fully qualified reference; forward references work |
| `Box<T>` | The type `T`; Rust storage indirection is not a Morphir type |

The frontend accepts `bool`, `char`, `String`, `i64`, `f64`, `Vec<T>`,
`Option<T>` and `Result<T, E>`. Local declarations and parameters take precedence
over builtin short names. It checks generic arity, unbound names, duplicates,
name-normalization collisions and alias cycles. Names currently require ASCII
Rust identifiers. Documentation is preserved; derives from the explicit builtin
allowlist are accepted without generating their implementations. The allowlist
is `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash`, `Default`.

Imports, module declarations, dependency compilation, references, lifetimes,
traits, bounds, const generics, default type parameters, enum discriminants,
macro invocations and other attributes are rejected. Mixed public/private struct
fields are rejected. Syn is a syntax parser: successful conversion does not claim
that rustc has checked the original source. Use `cargo check` on source projects
when compiler validation is needed.

With `typesOnly: true`, standalone function items are omitted with a warning.
With `typesOnly: false`, encountering a function fails. Constants, statics and
implementation blocks are unsupported. Failure returns diagnostics without
partial IR. Source diagnostics carry URI and zero-based UTF-16 positions.

`irVersion` accepts `3`, `4` and compatible exact release spellings according to
the core support table. Output uses the baseline format and reports `3` or `4`.
Unsupported major/minor versions fail. CLI context options `outputDir`,
`sourceRootUri`, `sourceRoot`, `emitParseStage`, `emitParseStageFatal` are validated.
Parse-stage output is unavailable: a request warns or fails when marked fatal.
Unknown options fail.

## Generated Rust

The backend accepts v3 Library distributions and v4 Library, Specs and
Application distributions. It returns a single relative `lib.rs` artifact with
nested modules. Values are omitted with a warning; incomplete v4 definitions
fail. Private definitions are retained for internal references. No files are
written by the extension.

| IR type | Rust representation |
| --- | --- |
| Variable | Generic type parameter |
| Reference | Local generated type, SDK mapping, or configured external path |
| Tuple / Unit | Rust tuple / `()` |
| Named record alias | Named struct |
| Anonymous record | Generated helper struct |
| Extensible record | Known fields plus `remaining_fields: R` for row parameter `R` |
| Function | `std::rc::Rc<dyn std::ops::Fn(A) -> B>` |
| Custom type | Enum, with boxed references where the type dependency graph is cyclic |
| Private custom constructors | Public type wrapper with inaccessible representation |
| Opaque specification | Required explicit external Rust type binding |
| Derived specification | Nominal wrapper over its base type; conversion names in documentation |

Phantom parameters use `PhantomData` only where Rust requires it. Empty generic
custom types contain `Infallible` so generation does not add inhabitants. IR
aliases with unused parameters use an associated-type helper, preserving the
aliased Rust type. Rust source aliases with unused parameters are rejected by
the frontend because they are invalid Rust.

Records become nominal Rust structs; structural equality and row-polymorphic
operations are not implemented in this type-only increment. Row payloads do not
enforce disjoint field sets. Function values are single-threaded `Rc` callables;
code generation does not insert `Send` or `Sync` requirements. Conversion bodies
for derived types are deferred with value support.

| Morphir SDK type | Rust type |
| --- | --- |
| `basics#bool`, `basics#int`, `basics#float` | `bool`, `i64`, `f64` |
| `char#char`, `string#string` | `char`, `String` |
| `list#list A`, `maybe#maybe A` | `Vec<A>`, `Option<A>` |
| `result#result E A` | `Result<A, E>` |
| `dict#dict K V`, `set#set A` | `BTreeMap<K, V>`, `BTreeSet<A>` |
| `basics#order`, `basics#never` | `Ordering`, `Infallible` |

These references use package `morphir/SDK`. `i64` is a finite-width target mapping;
the binding does not claim arbitrary-precision integer value semantics. Container
operations and their trait requirements are outside this type-only increment.
Other external references require an explicit mapping:

```json
{
  "externalTypes": {
    "vendor/types:containers#box": "std::boxed::Box",
    "vendor/domain:identity#id": "my_domain::Identity"
  }
}
```

Mappings are Rust paths without generic arguments. IR arguments are appended in
their declared order. The consumer supplies external crates and implementations.
Mappings do not synthesize opaque representations. Unknown types without a
mapping, wrong arities, invalid names and Rust name collisions fail generation.

## Native usage and tests

The public entry point is `RustExtension`, implementing the SDK's `Extension`,
`Frontend` and `Backend` traits. Wrap it in
`NativeExtension::frontend_backend(RustExtension)` to use JSON-RPC dispatch.
See [the native pipeline test](tests/pipeline.rs) for complete requests and
[the acceptance driver](tests/acceptance.rs) for generated-code consumer checks.

Run from the `morphir-rust` workspace:

```console
cargo test --locked -p morphir-rust-binding
cargo clippy --locked -p morphir-rust-binding --all-targets -- -D warnings
cargo build --locked -p morphir-rust-binding --target wasm32-unknown-unknown
cargo test --locked -p morphir-rust-binding --features wasm-host-tests --test wasm -- --ignored
```

For a distributable guest, build with `--release`. Its artifact is
`target/wasm32-unknown-unknown/release/morphir_rust_binding.wasm`. Set
`MORPHIR_RUST_GUEST` to that artifact's absolute path when running the host tests. Tests load it with
WASI disabled and compare native and guest results for both IR versions.

The guest reports ID `morphir-rust`, name `Morphir Rust`, version `0.1.0`,
frontend language `rust`, suffix `.rs`, backend target `rust`, and
`irVersions: ["3", "4"]` on both capabilities. Release bundle packaging and
CLI installation coverage are follow-on work; this increment verifies the
guest directly through the extension protocol.

Round-trip tests compare supported module declarations, not source formatting,
derive implementations, Rust ownership, or ignored values. Generated crate
module wrappers and backend-only helper types exceed the frontend's one-module
subset. Value compilation, imports and whole-crate name resolution are follow-on
work.
