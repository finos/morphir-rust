# Python models and conditional functions

`morphir-python-binding` implements the `morphir-python` extension in Rust.
It provides MEP `frontend/compile` and `backend/generate` through the Morphir
extension SDK, both natively and as an Extism WebAssembly guest. Python source
is parsed with Ruff and never imported or executed. The backend builds
declarations, parses them into Ruff's AST, and uses Ruff's generator to write
Python. All four direct Ruff component dependencies are pinned together.

This is an initial implementation of a defined subset of Python 3.12 and later
and Morphir IR v4. It supports ADT declarations, fixed tuples and annotated pure conditional
functions. It does not implement all Python syntax or all Morphir IR nodes, and
does not claim full Morphir Compatibility Kit conformance for this extension.
It has no dependency on `finos/morphir-python`.

The examples below describe this checkout. Build the extension from source and
use the [local installation guide](../../docs/tutorials/python-extension.md).

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
The frontend rejects its imports, calls, assignments and assertions.

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
| Local type annotation | Fully qualified reference in the current package and module |
| Annotated function | Value definition with typed parameters and an expression body |
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

Each request contains exactly one `.py` source document. Its filename determines
the module name, so `models.py` becomes `models`. The package name must be a
canonical Morphir package path such as `acme/example`. An empty `exposedModules`
exposes this module; otherwise the list must name this module alone. All emitted
types, constructors and functions are public. Declaration names use Morphir's
existing name codec, including uppercase initialism segments. Collisions after
normalization are errors. Names must be ASCII identifiers without a leading
underscore; Python keywords and the builtins used by this subset are reserved.
Module filenames also exclude Windows device names such as `con` and `aux`.
Names are checked in their generated spelling too: a field named `Class` would
become the reserved word `class`, so it is rejected. `zip_code` becomes the
Morphir name `zip-code` and generates back as `zip_code`. Type and constructor
names generate in PascalCase; field, function and parameter names use snake_case.

The backend returns one relative `.py` artifact. It rejects names that cannot
roundtrip through this spelling, including collisions across types, constructors
and functions after Morphir name normalization. The common Morphir pattern where
a type and its single constructor share a name therefore needs distinct names
for this first Python subset.

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
`bool`, `int`, `float` or `str` literals, fixed tuples, comparisons, and conditional
expressions.
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

Function and parameter names generate as snake_case. Integer literals preserve
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

The frontend emits explicit parameter and return types, but does not attach
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

Both directions use `morphir-core::ir::v4` and its JSON codec. The frontend emits
`formatVersion: 4` and canonical names; the backend reads supported v4 documents
through that codec. There is no v1-v3 migration in this extension.

| IR area | Frontend emits / backend accepts |
| --- | --- |
| Distribution | `Library`, exactly one module, empty dependency map, public definitions |
| Type definitions | Non-generic record aliases, fixed tuple aliases, custom types with public constructors |
| Type expressions | The four SDK scalar references, local type references, fixed tuples; a record at a record-alias body |
| Value definitions | `ExpressionBody` with annotated inputs and a required output type |
| Value expressions | Parameter `Variable`, scalar `Literal`, fixed `Tuple`, `IfThenElse`, and fully applied two-argument SDK scalar comparisons |
| Literal kinds | `BoolLiteral`, arbitrary-precision `IntegerLiteral`, finite `FloatLiteral`, `StringLiteral` |
| Metadata | Default/empty node attributes; documentation and non-empty retained type/value attributes are rejected |

Unsupported IR includes `Specs` and `Application` distributions, dependencies,
private definitions, generic types, opaque types, empty custom types, extensible
records, unit and function types, free type variables, and other SDK types such
as List, Maybe and Decimal. Arbitrary aliases such as an alias directly to `int`
are not supported. Local references must resolve within the supported module.

Unsupported value forms include record construction or field access,
constructor application, general calls or references, lists, arithmetic,
lambdas, let bindings, pattern matching, record updates, holes, and
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
| [Acceptance scenarios](tests/features/adt.feature) | Supported models pass through the public extension API; unsupported input returns diagnostics |
| [WASM host test](../morphir-daemon/tests/python_extension.rs) | Capability negotiation and compile/generate/recompile through an actual Extism guest; run by CI |
| Example verification | Generated example IR checked against the v4 JSON Schema, and selected generated Python branch/tuple results checked in Python 3.14.7 |

The repository's [Kit conformance job](../../.github/workflows/ci.yml) runs the
shared MCK against the Rust adapter. It does not certify the Python extension's
language mapping. There is currently no complete Python-extension MCK report or
full Python/Morphir semantic equivalence claim. Generated Python type annotations
also do not enforce argument types at runtime.

## Current boundary

Supported fields are scalars, local references and fixed tuples of at least two
elements. The two imports shown above, frozen dataclasses, non-generic
`type` aliases of dataclass variants or fixed tuples, and annotated pure functions
are the accepted module statements.
Comments and whitespace are not preserved. Methods, field defaults,
inheritance, arbitrary decorators or imports, docstrings, generic parameters,
containers, optional fields, quoted annotations and multi-module compilation
are rejected. Function calls, constructor calls, assignments, loops, bare returns,
decorated or async functions, parameter defaults, variadic parameters,
positional-only or keyword-only parameters, chained comparisons, and boolean
operators are not supported yet. The backend also rejects private definitions, documentation,
attributes, dependencies and IR type forms outside this subset. Failures return
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
sources accept either value. `irVersion` accepts `4` or `4.0.0`. The CLI's
`outputDir` and `sourceRootUri` string options are accepted as context, without
filesystem access. `emitParseStage=true` produces warning `PY006`; combining it
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
cargo test --locked -p morphir-daemon --test python_extension -- --ignored --exact python_adt_and_conditional_roundtrip_through_the_real_wasm_extension
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
`extension/python/v0.1.0` publish the same tested bundle as GitHub release assets.
The descriptor declares language `python` (`.py`), target `python`, MEP `0.1`
and IR `4`; these declarations cover only the subset documented above. The
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
