# Python models, functions and lambdas

`morphir-python-binding` implements the `morphir-python` extension in Rust.
It provides MEP `frontend/compile` and `backend/generate` through the Morphir
extension SDK, both natively and as an Extism WebAssembly guest. Python source
is parsed with Ruff and never imported or executed. The backend builds
declarations, parses them into Ruff's AST, and uses Ruff's generator to write
Python. All four direct Ruff component dependencies are pinned together.

This is an initial implementation of a defined subset of Python 3.12 and later
and Morphir IR v3 and v4. It supports ADT declarations, fixed tuples and annotated pure conditional
functions, typed calls and unary lambdas. It does not implement all Python syntax or all Morphir IR nodes, and
does not claim full Morphir Compatibility Kit conformance for this extension.
It has no dependency on `finos/morphir-python`.

The examples below describe the published `extension/python/v0.4.0` bundle. The earlier
`extension/python/v0.1.0` bundle does not include multi-module, private-module, IR v3 or higher-order
function support, and Morphir CLI `0.4.0-beta.1` and later cannot compile with it. Use the
[installation guide](../../docs/tutorials/python-extension.md) to install the bundle.

## A complete model

Save this as `models.py`. It is valid Python and supported frontend input:

```python
from __future__ import annotations
from dataclasses import dataclass

@dataclass(frozen=True)
class Pending:
    pass

@dataclass(frozen=True)
class Approved:
    amount: float

type Decision = Pending | Approved

@dataclass(frozen=True)
class Application:
    decision: Decision
    verified: bool

def choose(flag: bool, first: Decision, second: Decision) -> Decision:
    return first if flag else second
```

`Decision` becomes a custom type with `Pending` and `Approved` constructors.
`Application` becomes a record type alias. `choose` accepts and returns a
`Decision`; it selects an existing value. Constructing an `Approved` inside a
compiled function is not supported yet.

After importing the generated module in a Python application, callers can
instantiate its dataclasses and call its functions:

```python
from models import Approved, Pending, choose

approved = Approved(amount=125.0)
pending = Pending()
assert choose(True, approved, pending) is approved
assert choose(False, approved, pending) is pending
```

This caller is ordinary application code, not input to the Morphir frontend.
The frontend rejects its constructor calls, assignments and assertions.

## Mapping

| Python declaration | Morphir definition |
| --- | --- |
| Frozen dataclass outside a sum | Type alias whose body is a record |
| `type Decision = Pending \| Approved` | Custom type with named constructors |
| Dataclass named in that alias | Constructor with its fields as ordered parameters |
| Variant with `pass` | Constructor with no parameters |
| `int`, `float`, `bool`, `str` | SDK `basics#int`, `basics#float`, `basics#bool`, `string#string` |
| `tuple[A, B]` | Tuple type |
| `type Point = tuple[float, float]` | Type alias with a tuple body |
| `(x, y)` or `return x, y` | Tuple value with ordered elements |
| Local or imported type annotation | Fully qualified reference to its defining module in the current package |
| Annotated function | Value definition with typed parameters and an expression body |
| `Callable[[A], B]` | Unary `Function` type from `A` to `B` |
| Named function / function call | Same-package `Reference` / ordered `Apply` nodes |
| `lambda item: body` | `Lambda` with a named wildcard `AsPattern` and a lexically scoped body |
| Returning `if`/`elif`/`else`, or `a if condition else b` | `IfThenElse` with `condition`, `then`, and `else` members |

The scalar references are exactly `morphir/SDK:basics#int`,
`morphir/SDK:basics#float`, `morphir/SDK:basics#bool`, and
`morphir/SDK:string#string`. With package `acme/example` and file `models.py`,
`Decision` is referenced as `acme/example:models#decision`.

A variant belongs to exactly one sum. It is not also emitted as a record alias,
and its class name cannot be used as a field type. Refer to its enclosing sum
instead. A one-variant sum is spelled `type Wrapped = Variant`.
Recursive fields work with postponed annotations. These mappings preserve
declarations; Python itself does not enforce annotation types at runtime.

Each request contains one or more `.py` source documents. A source-relative path
determines its module name: `models.py` becomes `models`, and
`domain/order_items.py` becomes `domain/order-items`. The package name must be a
canonical Morphir package path such as `acme/example`. Omitted `exposedModules`
exposes every module. An explicit empty list makes every module private; a
nonempty list selects the public modules, using canonical paths or dotted names
such as `Domain.OrderItems`. Other modules remain private and usable by siblings.
Unknown exposure names are errors. All emitted types, constructors and functions
within those modules are public. Declaration names use Morphir's
existing name codec, including uppercase initialism segments. Collisions after
normalization are errors. Names must be ASCII identifiers without a leading
underscore; Python keywords and the builtins used by this subset are reserved.
Module filenames also exclude Windows device names such as `con` and `aux`.
Names are checked in their generated spelling too: a field named `Class` would
become the reserved word `class`, so it is rejected. `zip_code` becomes the
Morphir name `zip-code` and generates back as `zip_code`. Type and constructor
names generate in PascalCase; field, function and parameter names use snake_case.

The backend returns one relative `.py` artifact per module. It rejects names that cannot
roundtrip through this spelling, including collisions across types, constructors
and functions after Morphir name normalization. The common Morphir pattern where
a type and its single constructor share a name therefore needs distinct names
for this first Python subset.

## Multiple modules and imports

Module privacy is package metadata, not Python source syntax. Both the frontend
and backend support public and private modules. For a project with
`src/domain/models.py` and `src/domain/rules.py`, this configuration exposes only
the rules module:

```toml
[project]
name = "acme/example"
version = "0.1.0"
source_directory = "src"
exposed_modules = ["domain.rules"]

[frontend]
language = "python"
```

`rules.py` may import types from the private `models.py`. The backend generates
both files. Keep the exposure configuration when recompiling generated files to
preserve their Morphir visibility; ordinary Python imports do not enforce that
visibility at runtime. Private types, constructors, and functions are still
outside the supported subset.

Compile all source files together in one request. For example, save the complete
model above as `models.py` and this function as `rules.py`:

```python
from __future__ import annotations
from models import Decision as Result

def select_decision(flag: bool, first: Result, second: Result) -> Result:
    if flag:
        return first
    else:
        return second
```

The function's parameter and result types reference
`acme/example:models#decision`. Import aliases do not create new IR types.
Imported record, sum and tuple types work in fields, constructor parameters and
function signatures. Tuple aliases resolve across modules when checking returns.
The order of source documents does not affect the resulting IR.

Supported imports include `from models import Decision`, optional `as` aliases,
`import models`, `import domain.models as model`, and relative imports such as
`from .models import Decision` or `from ..models import Decision` inside nested
modules. `from domain import models` also binds a source module. Imports must
resolve to files supplied in the request, and named type or function imports must refer to
declarations in that module. Importing constructors or re-exported
names, wildcard imports, lazy imports and external package imports are rejected.
The standard imports `dataclasses.dataclass`, `__future__.annotations`, and
`Callable` from `typing` or `collections.abc` are supported. A sum's variant
classes must stay in the same module as its alias.

Relative document paths are relative to the source root. For multiple absolute
paths or file URIs, set `sources.root`; the CLI does this when
compiling a directory. URI queries and fragments do not affect module identity;
percent-encoded path segments are decoded before validating module names.
Dot segments and encoded path separators are rejected. Absolute documents outside that root, module paths that
collide after normalization or case folding, and a file that is also a package directory such as
`models.py` alongside `models/item.py` are errors. A single absolute document
without a root retains the original filename-based behavior.

Nested modules use Python namespace packages. `__init__.py` modules and package
initialization code are not supported. Generated paths preserve the directory
structure, and generated imports use absolute module paths with collision-free
aliases. Put the generated directory on Python's import path. The backend uses
module imports and postponed annotations, so mutually referring record types
can be generated without eager cross-module type imports. Recursive tuple aliases
remain unsupported. No imports execute during compilation or generation.

Run `cargo run -p morphir-python-binding --example python_modules -- 3` for a v3
example, or omit the final argument for v4. Both provide a complete
native MEP example with imported ADTs and tuple aliases.

## Functions and lambdas

Both IR v3 and v4 support these declarations:

```python
from collections.abc import Callable

def identity(value: int) -> int:
    return value

def apply(transform: Callable[[int], int], value: int) -> int:
    return transform(value)

def retain(value: int) -> Callable[[int], int]:
    return lambda ignored: value

def run(value: int) -> int:
    return apply(lambda item: identity(item), value)
```

Functions may call other annotated functions in the supplied package, including
forward references, recursion, and functions imported from private modules.
Calls are checked against the declared parameter and return types. Lambdas can
capture enclosing parameters and shadow a parameter with the same source name.
Different spellings that would collapse two lexical bindings to the same Morphir
name are rejected. Generated global function references use module aliases that
avoid parameter and lambda names, including when referencing the current module.
Keep generated files together and import them as modules when executing them.

Import `Callable` from `collections.abc` or `typing`, optionally with an alias;
qualified forms such as `typing.Callable` also work after importing the module.
`Callable` and `callable` are reserved declaration names. Lambda input types come
from an annotated return, a function argument, or a containing tuple/conditional.
An immediately called lambda can infer its input from its argument.

Morphir function types are unary. First-class function values therefore use
exactly one parameter: `Callable[[A], B]`, a unary named function, or a unary
lambda. For currying, nest the types and lambdas explicitly:

```python
def select(flag: bool) -> Callable[[int], Callable[[int], int]]:
    return lambda first: lambda second: first if flag else second

def selected(value: int) -> int:
    return select(True)(value)(0)
```

Ordinary `def` functions may still have multiple parameters, but their direct
calls must supply all arguments positionally. A zero-parameter `def` call maps
to a value `Reference` and generates as a zero-argument call. Zero-parameter or
multi-parameter definitions cannot themselves be passed as callable values.
Partial applications of multi-parameter definitions, multi-argument `Callable`
annotations or lambdas, keyword arguments, starred arguments, defaults,
variadics, untyped lambda inputs without context, and lambda destructuring
patterns are outside this binding's subset. Nested `def`, assignments and
general Python inference remain unsupported.

The [function fixture](tests/fixtures/functions.py) is complete frontend input.
The [function tests](tests/functions.rs) cover structural round trips, independently
authored IR and executable Python results. CI also sends these functions through
the packaged WASM extension in both IR versions.

## Conditional function bodies

This function can be added to `models.py`:

```python
def classify(value: int) -> str:
    if value < 0:
        return "negative"
    elif value == 0:
        return "zero"
    else:
        return "positive"
```

Functions need parameter and return type annotations. They may return parameters,
`bool`, `int`, `float` or `str` literals, fixed tuples, comparisons, conditional
expressions, typed calls, function references and contextual lambdas.
Parameters can also carry local ADTs or fixed tuples. Conditions must have type
`bool`; both branches must have the same type and match the declared return type.
Scalar comparisons `==`, `!=`, `<`, `<=`, `>` and `>=` map respectively to SDK
Basics `equal`, `not-equal`, `less-than`, `less-than-or-equal`, `greater-than`, and
`greater-than-or-equal`. Each is a `Reference` with two nested `Apply` nodes.
Operands must have matching scalar types; booleans support equality only.

Nested branches and `elif` chains work. An early-return branch can use a following
return as its fallback, for example `if flag: return first` followed by
`return second`. Every path must return a value. The backend generates explicit
returning `if`/`else` blocks, preserving which branch is evaluated. It may use a
conditional expression inside a condition or comparison operand.

Function and parameter names generate as snake_case. For IR v4, integer literals preserve
arbitrary precision, including decimal, hexadecimal, octal and binary source
literals. The backend emits decimal integers. Float literals must be finite. Python's implicit
truthiness and numeric coercions are not part of this subset. These restrictions
are checked during compilation and generation.

The `choose` function in the complete model has this value definition at
`values["choose"]["Public"]`, when compiled with package `acme/example`:

```json
{
  "ExpressionBody": {
    "inputTypes": {
      "flag": "morphir/SDK:basics#bool",
      "first": "acme/example:models#decision",
      "second": "acme/example:models#decision"
    },
    "outputType": "acme/example:models#decision",
    "body": {
      "IfThenElse": {
        "condition": {"Variable": "flag"},
        "then": {"Variable": "first"},
        "else": {"Variable": "second"}
      }
    }
  }
}
```

This shows the extension's direct output. A host such as the CLI may write the
equivalent expanded type encoding, for example
`{"Reference": {"fqname": "morphir/SDK:basics#bool"}}` in place of the compact
string. Such spelling changes do not change the type.

For v4, the frontend emits explicit parameter and return types, but does not attach
inferred-type or source-location attributes to each IR value node. Its expression
checks cover the supported subset; they are not a general Morphir type checker.

## Tuples

This code can also be added to `models.py`:

```python
type Point = tuple[float, float]
type LabeledPoint = tuple[Point, str]

def make_point(x: float, y: float) -> Point:
    return (x, y)

def located(flag: bool, location: Point) -> LabeledPoint:
    if flag:
        return (location, "known")
    else:
        return ((0.0, 0.0), "unknown")
```

Fixed tuples of two or more elements work in dataclass fields, constructor
parameters, function parameters, return types and function expressions. Elements
can be scalars, local types or nested tuples. Named tuple aliases are expanded
when checking expression types, while their names are retained in IR signatures.
Each tuple element and its position must match the annotation.

The `Point` alias has `TypeAliasDefinition.typeExp`:

```json
{"Tuple": ["morphir/SDK:basics#float", "morphir/SDK:basics#float"]}
```

The body of `make_point` is a value expression:

```json
{"Tuple": [{"Variable": "x"}, {"Variable": "y"}]}
```

The codec writes the tuple type wrapper shown here; it also accepts the IR v4
compact bare-array spelling in type positions. A tuple **value** always has a
`Tuple` wrapper. In value positions, a bare array means a list and is outside
this binding's subset.

Empty tuples, one-element tuples, `tuple[T, ...]`, unparameterized `tuple`,
`typing.Tuple`, `NamedTuple`, starred expansion, tuple indexing, destructuring,
tuple comparison and recursive tuple aliases are not supported. These are limits
of this binding, not claims that Morphir IR cannot represent those concepts.

## Morphir IR support and conformance

Both directions support IR v3 and v4 through the shared `morphir-core` models.
The frontend uses one v4 language model and encodes the requested version. The
backend detects the document version, migrates v3 through the shared core API,
and applies the same Python subset checks. IR v1 and v2 are unsupported.
Readers follow the shared compatibility intervals `[3.0.0,3.2.0)` and
`[4.0.0,4.1.0)`, accepting later patches but rejecting later minor versions.
Compilation targets the baseline versions 3.0.0 and 4.0.0.

| Version | Encoding and attributes |
| --- | --- |
| v3 | `formatVersion: 3`, classic tagged arrays and word-array names; function values carry checked inferred types |
| v4 | `formatVersion: 4`, canonical names and the v4 JSON codec; emitted node attributes remain empty |

V3 generation accepts typed value annotations, checks them against the supported
expression types, and regenerates them on recompilation. Empty documentation
wrappers normalize to absent documentation. Other documentation and metadata
remain unsupported. Untyped v3 function bodies are not accepted by the current
typed distribution reader. V3 whole-number literals must fit the shared codec's
signed 64-bit range, from -9223372036854775808 to 9223372036854775807. Larger
Python integer literals produce a diagnostic for v3 and remain supported in v4.
These are current binding/codec limits, not limits attributed to Morphir IR.
V3 compilation also checks that every name survives the shared legacy name
conversion. For example, `a_b` would become the initialism `AB`, so it is rejected
with `PY003` for v3; rename it or target v4. Initialisms such as `APIResponse`
remain supported.

The type and value subset below applies to both versions; the displayed JSON
examples elsewhere in this README use v4.

| IR area | Frontend emits / backend accepts |
| --- | --- |
| Distribution | `Library`, one or more public or private modules, empty dependency map; definitions within modules are public |
| Type definitions | Non-generic record aliases, fixed tuple aliases, custom types with public constructors |
| Type expressions | The four SDK scalar references, same-package type references across modules, fixed tuples, unary `Function`; a record at a record-alias body |
| Value definitions | V4 `ExpressionBody`, or the classic v3 value-definition object, with annotated inputs and a required output type |
| Value expressions | Parameter `Variable`, scalar `Literal`, fixed `Tuple`, `IfThenElse`, same-package function `Reference`, typed `Apply`, unary `Lambda`, and fully applied two-argument SDK scalar comparisons |
| Lambda patterns | `AsPattern` over `WildcardPattern`, binding one parameter; v3 pattern annotations are checked |
| Literal kinds | `BoolLiteral`, finite `FloatLiteral`, `StringLiteral`; v4 arbitrary-precision `IntegerLiteral` or v3 signed 64-bit `WholeNumberLiteral` |
| Metadata | Empty type attributes; v4 empty value attributes or v3 checked inferred types. Nonempty documentation and other retained metadata are rejected |

Unsupported IR includes `Specs` and `Application` distributions, dependencies,
private types, constructors and values, generic types, opaque types, empty custom types, extensible
records, unit types, free type variables, and other SDK types such
as List, Maybe and Decimal. Arbitrary aliases such as an alias directly to `int`
are not supported. Type references must resolve within the supplied package.

Unsupported value forms include record construction or field access,
constructor application, external calls or references, lists, arithmetic,
let bindings, general pattern matching, record updates, holes, and
`NativeBody`, `ExternalBody` or `IncompleteBody` definitions. A valid Morphir IR
document can therefore still be outside this extension's supported subset.

For supported models, the round-trip contract is preservation of the model
through Python → IR → Python → IR. Tests compare compiled IR structurally and
also exercise independently authored IR inputs. This is not source-text or
byte-for-byte JSON preservation: formatting, comments, declaration spelling,
declaration order and accepted alternate JSON encodings may normalize. Numeric
literals may be respelled. The extension does not preserve arbitrary metadata
that the shared IR reader ignores.

Current evidence is scoped to the checked-in cases:

| Evidence | What it establishes |
| --- | --- |
| [ADT integration tests](tests/pipeline.rs) | Expected IR mappings, independent custom-type input, recursive references, naming and rejection boundaries |
| [Function and tuple integration tests](tests/conditionals.rs) | Exact `IfThenElse`/tuple encodings, alias expansion, branch/type validation and round-trips |
| [Functions and lambdas](tests/functions.rs) | V3/v4 calls, closures, independent IR, annotation and scope checks; opt-in Python execution runs in CI |
| [Version integration tests](tests/ir_versions.rs) | V3/v4 roundtrips, private imports, independently authored v3 input, integer bounds and annotation validation |
| [Module integration tests](tests/modules.rs) | Absolute and relative imports, nested modules, cyclic record references, import collisions and cross-module tuple checking |
| [Acceptance scenarios](tests/features/adt.feature) | Supported models pass through the public extension API; unsupported input returns diagnostics |
| [WASM host test](../morphir-host-native/tests/python_extension.rs) | Capability negotiation and compile/generate/recompile through an actual Extism guest; run by CI |
| Example verification | Generated example IR checked against the v4 JSON Schema, and selected generated Python branch/tuple results checked in Python 3.14.7 |

The repository's [Kit conformance job](../../.github/workflows/ci.yml) runs the
shared MCK against the Rust adapter. It does not certify the Python extension's
language mapping. There is currently no complete Python-extension MCK report or
full Python/Morphir semantic equivalence claim. Generated Python type annotations
also do not enforce argument types at runtime.

## Current boundary

Supported fields are scalars, same-package references, unary `Callable` types and
fixed tuples of at least two elements. The imports described above, frozen dataclasses, non-generic
`type` aliases of dataclass variants or fixed tuples, and annotated pure functions
are the accepted module statements.
Comments and whitespace are not preserved. Methods, field defaults,
inheritance, arbitrary decorators or imports, docstrings, generic parameters,
containers, optional fields, quoted annotations and cross-package dependencies
are rejected. Constructor calls, assignments, loops, bare returns,
decorated or async functions, parameter defaults, variadic parameters,
positional-only or keyword-only parameters, chained comparisons, and boolean
operators are not supported yet. The backend also rejects private types, constructors and values, documentation,
unsupported metadata, dependencies and IR type forms outside this subset. Failures return
diagnostics with no partial IR or artifacts.

For example, this is valid Python but rejected with `PY004`, because an integer
condition would use Python's implicit truthiness:

```python
def invalid(value: int) -> int:
    return 1 if value else 0
```

Other rejected examples are `return (1, 2)` for `-> tuple[int, str]`, an `if`
whose false path reaches the end of the function without a return, and
`type Point = tuple[float, ...]`. No incomplete model is emitted for these cases.

Compile functions with `typesOnly=false`. A request containing functions with
`typesOnly=true` returns a diagnostic; bodies are never silently dropped. ADT-only
sources accept either value. `irVersion` accepts `3`, `3.0.0`, `4` or `4.0.0`. The CLI's
`outputDir` string option is accepted as context; `sources.root` determines
module paths for absolute document URIs. Neither grants filesystem access.
`emitParseStage=true` produces warning `PY006`; combining it
with `emitParseStageFatal=true` fails. Other options are rejected.

| Code | Meaning |
| --- | --- |
| `PY001` | Unsupported request or option |
| `PY002` | Python syntax error, with a source range |
| `PY003` | Invalid name or naming collision |
| `PY004` | Unsupported declaration, annotation or IR feature |
| `PY005` | IR decoding or serialization, or generated syntax failure |
| `PY006` | Parse-stage output was requested but is unavailable |

Semantic lowering diagnostics identify the source module; request-option
diagnostics can have no source location. Syntax diagnostics
identify the parser's failing range. Source positions use the SDK's UTF-16
convention.

## Run the pipeline

From the `morphir-rust` checkout:

```console
cargo test --locked -p morphir-python-binding
cargo run --locked -p morphir-python-binding --example python_adt
rustup target add wasm32-unknown-unknown
cargo build --locked --release -p morphir-python-binding --target wasm32-unknown-unknown
cargo test --locked -p morphir-host-native --test python_extension -- --ignored --exact python_adt_and_conditional_roundtrip_through_the_real_wasm_extension
```

The example prints JSON containing compiled IR and generated source for the
[ADT](tests/fixtures/models.py), [conditional](tests/fixtures/conditionals.py), and
[tuple](tests/fixtures/tuples.py) fixtures. Its values and names differ from the
shorter README model. To compile the README model directly, save its complete
model block as `models.py` and run:

```console
cargo run --locked -p morphir-python-binding --example python_adt -- models.py
```

The optional path selects one source file; the example uses package `acme/example`
and returns IR plus generated Python through native MEP. File reading belongs to
this example program, not the extension. The integration test invokes both
capabilities through the actual WASM guest.
`MORPHIR_PYTHON_WASM` can select a different guest file for that test.
The native `NativeExtension::frontend_backend(PythonExtension)` adapter exposes
the same MEP methods without requiring a WASM runtime.

Build the installable release bundle with `mise run extension:artifact:python`.
This runs native tests, builds and validates the WASM guest, packages its checksum
and descriptor, and tests publication, installation and both capabilities with a
fresh Morphir Home after removing the source repository. The three bundle files
are written to `.morphir/build/extensions/python/`.

CI uploads these files as `morphir-python-extension-bundle`. Tags such as
`extension/python/v0.4.0` publish the same tested bundle as GitHub release assets.
The descriptor declares language `python` (`.py`), target `python`, MEP `0.1`
and IR `3` and `4`; these declarations cover only the subset documented above. The
release is a Morphir WASM extension, not a PyPI package or a CPython import module.

For installation with the parent Morphir CLI, see the
[Python extension guide](../../docs/tutorials/python-extension.md).

## Specification sources

The implementation uses `morphir-core::ir::v4` rather than a separate IR model.
The parent repository's [IR knowledgebase](https://github.com/finos/morphir/tree/main/kb/bundles/morphir/morphir-ir)
records the naming, record-field, constructor-parameter and SDK package decisions.
The [IR specification](https://morphir.finos.org/docs/spec/ir/) and the
[extension tutorial](../../docs/tutorials/extension-tutorial.md) describe the
contracts this crate uses. Ruff's [parser](https://github.com/astral-sh/ruff/tree/main/crates/ruff_python_parser)
and [generator](https://github.com/astral-sh/ruff/tree/main/crates/ruff_python_codegen)
remain internal component crates; upgrades require running both native and WASM tests.

### Bundle descriptor compatibility

Newly built bundles include a version-2 `release.json` with capability claims
read from the exact WASM being packaged and checked against
`.github/extensions.toml`. Publishing requires Morphir CLI `0.4.0-beta.7` or
later; older CLIs cannot read this descriptor. The versioned WASM and
`.wasm.sha256` names are unchanged. The downloaded descriptor is still named
`<artifact-base>-<version>.release.json`; rename it to `release.json` before
publishing. WASM installs show `Claims: unchecked`.

See the [packaging contract](../../docs/extensions/capability-claims.md#packaging-wasm-extensions)
for the descriptor shape and provenance rules.
