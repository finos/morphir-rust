# Rust types and pure functions

`morphir-rust-binding` implements the `morphir-rust` extension. It provides MEP
`frontend/compile` and `backend/generate` natively and as an Extism WASM guest.
Both capabilities advertise IR versions `3` and `4`, language/target `rust` and
source suffix `.rs`. Syn parses source without running rustc, macros or build
scripts. The shared `morphir-core` codecs and typed migration handle the IR.

The frontend and backend translate types and a small subset of pure
functions in both IR versions. The frontend also extracts explicitly annotated
native/external declarations for IR v4. It does not claim complete Rust language
support, all Morphir value semantics, or extension-level MCK conformance.

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
| Named enum payload | Constructor arguments in field declaration order |
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
macro invocations and attributes other than those documented here are rejected. Mixed public/private struct
fields are rejected. Syn is a syntax parser: successful conversion does not claim
that rustc has checked the original source. Use `cargo check` on source projects
when compiler validation is needed.

With `typesOnly: true`, standalone function items are omitted with a warning.
With `typesOnly: false`, ordinary functions compile within the subset below;
annotated bindings require IR v4. Constants, statics and
implementation blocks are unsupported. Failure returns diagnostics without
partial IR. Source diagnostics carry URI and zero-based UTF-16 positions.

`irVersion` accepts `3`, `4` and compatible exact release spellings according to
the core support table. Output uses the baseline format and reports `3` or `4`.
Unsupported major/minor versions fail. CLI context options `outputDir`,
`emitParseStage`, `emitParseStageFatal` are validated. Parse-stage output is
unavailable: a request warns or fails when marked fatal. Unknown options fail.
The Rust frontend compiles exactly one document and never reads `sources.root`,
so `sourceRootUri` and `sourceRoot` in the options bag are rejected like any
other unknown option, not silently accepted. A request in the transitional
legacy envelope — top-level `documents` rather than `sources` — never reaches
that check carrying one: the SDK moves the legacy root key into `sources.root`
while deserializing.

## Conditional functions in IR v3 and v4

Set `typesOnly: false` to compile ordinary Rust functions:

```rust
pub fn select(a: i64, b: i64) -> i64 {
    let larger = if a > b { a } else { b };
    if larger > 10 && !(a == b) { larger }
    else if a == b { 0 }
    else { -1 }
}
```

The backend generates callable Rust functions from this subset in either IR
version. Supported expressions are parameters and local variables, `i64`, finite
`f64`, Boolean and character literals, tuples, `()`, immutable named `let`
bindings, scalar comparisons, `!`, `&&`, `||`, nested `if/else`, and the `match` subset below. Blocks end in
a value expression, or yield Unit when empty. A missing `else` is valid only for
a Unit result. Boolean operators preserve short-circuit evaluation.

Conditions must be Boolean. Both branches and the declared return type must
agree. Integer literals must fit `i64`; only `i64` and `f64` numeric suffixes are
accepted. Comparisons support integers, floats and characters; Boolean operands
support `==` and `!=`. Borrowed string literals do not become owned `String`
values. Local shadowing is preserved with distinct IR names.

Functions can select and return existing values, including generic parameters and
domain types. Each generic parameter must occur in the signature; unused generic
parameters are rejected because IR cannot retain them. Non-Copy values have conservative move checks; returning one value
from each alternative branch is supported. Type aliases remain nominal during
expression checking, so alias-specific arithmetic, comparisons and literal
coercion are not supported. Built-in `Box` is rejected in executable signatures
and local annotations because its type-only erasure would lose Rust ownership
information.

The backend supports scalar, tuple, generic, named domain and function types in
function signatures. Anonymous structural records are rejected in executable
signatures; their standalone type declarations remain supported.

Arithmetic operators, loops, mutation, destructuring `let` bindings, early
`return`, and macro invocations are outside this increment. Ordinary
functions must be safe, synchronous and non-const, without an ABI, lifetimes or
trait bounds. Unsupported expressions and invalid types return diagnostics
without partial IR or generated artifacts. `typesOnly: true` continues to omit
ordinary functions with a warning without validating their bodies.

## Calls and lambdas in IR v3 and v4

Both directions support named calls, function values and typed lambdas:

```rust
pub fn positive(value: i64) -> bool { value > 0 }
pub fn invoke(predicate: fn(i64) -> bool, value: i64) -> bool {
    predicate(value)
}
pub fn above(limit: i64, value: i64) -> bool {
    let predicate = |n: i64| n > limit;
    predicate(value)
}
pub fn run(value: i64) -> bool { invoke(positive, value) }
```

The frontend resolves ordinary functions within the source module, including
forward references and unconstrained generic calls. Function pointer types and
aliases may appear in parameters, results and local annotations. Every lambda
parameter needs a type annotation; its result can be inferred or annotated.
Irrefutable tuple, wildcard and variable parameter patterns are supported.
Calls preserve Rust argument count, including zero-argument calls.
Callable aliases are expanded for expression checking; other aliases retain
the existing nominal checking rules.

Lambdas can capture immutable `i64`, `f64`, `bool`, `char`, Unit and tuples of
these types, and can be called repeatedly. A `move` lambda is accepted within
that same subset. Capturing lambdas cannot coerce to a Rust `fn` pointer;
noncapturing lambdas can. Mutable captures, owned non-Copy captures, captures of
function values, trait-based callables, methods and recursion are deferred.
Calls to annotated native/external declarations are also deferred.

IR uses standard Reference, Apply, Lambda and Function nodes. Function types are
curried, and zero-argument source callables use a Unit argument when represented
as function values. A reference to a zero-input IR definition evaluates that
definition's result. The backend emits named definitions with ordinary Rust
arguments and represents function values as `Rc<dyn Fn(A) -> B>`. Saturated
named calls use direct Rust calls; other applications invoke the generated
callable. Partial applications that would retain non-Copy arguments return a
diagnostic. Generated callable handles can be reused without consuming them.
The generated API therefore uses `Rc` callbacks even where source uses `fn`.
Multiargument lambdas also use nested IR lambdas, so parameters before the last
must have supported Copy types even when the source call supplies every argument.

## Pattern matching in IR v3 and v4

Both directions support ordered, exhaustive matches over local enums,
`Option<T>`, `Result<T, E>`, tuples, Boolean, `i64`, character and Unit values.
Patterns may nest and use wildcards or plain variable bindings. The subject is
evaluated once, and bindings are scoped to their arm:

```rust
pub fn amount(value: Option<Result<i64, bool>>) -> i64 {
    match value {
        Some(Ok(amount)) => amount,
        Some(Err(true)) => 1,
        Some(Err(false)) => -1,
        None => 0,
    }
}
```

Enum variants use qualified paths such as `Decision::Accepted(value)`. Named
variant fields are supported and lower in declaration order; field shorthand
and `..` are accepted. SDK variants accept short names (`Some`, `None`, `Ok`,
`Err`) or `Option::`/`Result::` qualification. Generic payload types are
substituted before checking arm bindings and results.

Every arm must return the same type. Exhaustiveness checks include nested
constructor and tuple cases. Integer, character and unconstrained generic
subjects need wildcard or variable coverage. A missing case is an error;
generation never inserts a panic fallback. Ownership checks remain conservative,
including consumption of a non-Copy match subject.

Guards, or-patterns, ranges, `@`, `ref`/`mut` bindings, reference and slice
patterns, floating-point and string patterns, empty matches, ordinary struct
patterns, and source patterns that require erased `Box` payloads are deferred.
Type aliases remain nominal for pattern checking. Unsupported patterns return
diagnostics without partial output.

The backend includes phantom generic fields in constructor patterns. Matching
private constructor wrappers, recursively boxed payloads and anonymous structural
payloads is deferred and returns a diagnostic; their type declarations remain
supported.

## IR v4 native and external declarations

Set `typesOnly: false` and `irVersion: "4"` to extract annotated function
signatures:

```rust
#[morphir::native(hint = "arithmetic", description = "Integer addition")]
pub fn add(a: i64, b: i64) -> i64 { a + b }

#[morphir::external(target = "rust", name = "vendor::lookup")]
#[morphir::external(target = "javascript", name = "store.lookup")]
pub fn lookup(id: i64) -> Option<String> { unimplemented!() }
```

`native` produces `NativeBody`, an operation whose implementation belongs to the
runtime. Supported hints are `arithmetic`, `comparison`, `string_op`,
`collection_op` and `platform_specific`. The last requires a nonblank `platform`
string; other hints reject that key. `description` is optional.

`external` produces `ExternalBody` with an explicit target platform and symbol
name. Repeat it for different targets. Both strings must be nonblank, and targets
must be unique. Symbols are opaque names, so the frontend does not resolve or
validate a Rust path. A declaration cannot combine native and external bindings.
Metadata accepts only the documented keys with string literal values.

These are source extraction attributes read by Syn. This crate does not provide
procedural macros that make the annotations compile with rustc. The frontend
parses function bodies for Rust syntax but does not execute or translate them,
and does not include them as an external fallback implementation.

Signatures use the same supported types and unconstrained type parameters as type
declarations. Parameters must be plain identifiers. Async, const, unsafe, ABI and
variadic functions, receivers, destructuring, `ref`/`mut` parameter bindings,
lifetimes and trait bounds are rejected. Visibility, documentation and parameter
order are preserved. Binding names cannot collide after normalization with other
bindings or module constructors.

IR v3 rejects these declarations with `RS_BINDING_VERSION`. With `typesOnly: true`,
both versions validate binding annotations and signatures, then omit them with
`RS_VALUES_UNSUPPORTED` warnings. The backend omits native and external
declarations with a warning; it does not generate their implementations.

## Generated Rust

The backend accepts v3 Library distributions and v4 Library, Specs and
Application distributions. It returns a single relative `lib.rs` artifact with
nested modules. Supported expression values become functions. Native, external,
incomplete and specification-only values are omitted with a warning; incomplete v4 type definitions
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
operations are not implemented in this increment. Row payloads do not
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
`irVersions: ["3", "4"]` on both capabilities.

## Release bundle

Build and validate the installable bundle with Python 3.11+ and the workspace's
mise tools available:

```console
mise run extension:artifact:rust
```

The task runs native tests, validates the release WASM guest, checks the guest
through MEP, and tests installation into a fresh Morphir home. The installed
extension compiles and generates executable Rust for IR v3 and v4 after its
source repository has been removed.

The bundle is written to `.morphir/build/extensions/rust/`: a versioned WASM
artifact, its SHA-256 checksum and `release.json`. A clean checkout builds from
an archived HEAD and records that commit in the descriptor. A dirty checkout
can produce a local test bundle, without release provenance.

Independent releases use tags such as `extension/rust/v0.2.0`. Download the
assets from the [Rust v0.2.0 release](https://github.com/finos/morphir-rust/releases/tag/extension/rust/v0.2.0).
The published descriptor is named `morphir-rust-binding-0.2.0.release.json`;
rename it to `release.json` alongside the WASM and checksum before publishing
the directory to a local extension repository.

Use the Rust Morphir CLI with frontend-bundle publication support; CLI
`0.4.0-alpha.6` and earlier cannot publish this descriptor. The npm
`morphir-elm` executable does not provide these installation commands.

```console
morphir extension repository init /absolute/path/to/rust-index
morphir extension repository add rust-local --directory /absolute/path/to/rust-index
morphir extension repository publish rust-local --bundle rust-bundle
morphir extension install --repository rust-local morphir-rust
```

Here `rust-bundle` is the downloaded bundle directory after renaming its
descriptor. Use the same Morphir home for installation, compilation and
generation. Hosts predating this release's daemon update have a smaller WASM
instruction budget and may reject larger supported source files with an
out-of-fuel error.

Round-trip tests compare supported module declarations, not source formatting,
derive implementations, Rust ownership, or ignored values. Generated crate
module wrappers and backend-only helper types exceed the frontend's one-module
subset. Additional expressions, imports and whole-crate name resolution are
follow-on work. Conditional, pattern and callable tests compile and execute source and
generated Rust against fixed expected results for both IR versions, including
the native and WASM extension protocols.
