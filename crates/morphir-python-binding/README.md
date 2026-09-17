# Python algebraic data types

`morphir-python-binding` implements the `morphir-python` extension in Rust.
It provides MEP `frontend/compile` and `backend/generate` through the Morphir
extension SDK, both natively and as an Extism WebAssembly guest. Python source
is parsed with Ruff and never imported or executed. The backend builds
declarations, parses them into Ruff's AST, and uses Ruff's generator to write
Python. All four direct Ruff component dependencies are pinned together.

This is an initial ADT subset for Python 3.12 and later. It reads and writes
Morphir IR v4 Library distributions. It has no dependency on `finos/morphir-python`.

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
```

## Mapping

| Python declaration | Morphir definition |
| --- | --- |
| Frozen dataclass outside a sum | Type alias whose body is a record |
| `type Decision = Pending \| Approved` | Custom type with named constructors |
| Dataclass named in that alias | Constructor with its fields as ordered parameters |
| Variant with `pass` | Constructor with no parameters |
| `int`, `float`, `bool`, `str` | SDK `basics#int`, `basics#float`, `basics#bool`, `string#string` |
| `tuple[A, B]` | Tuple type |
| Local type annotation | Fully qualified reference in the current package and module |

A variant belongs to exactly one sum. It is not also emitted as a record alias,
and its class name cannot be used as a field type. Refer to its enclosing sum
instead. A one-variant sum is spelled `type Wrapped = Variant`.
Recursive fields work with postponed annotations. These mappings preserve
declarations; Python itself does not enforce annotation types at runtime.

Each request contains exactly one `.py` source document. Its filename determines
the module name, so `models.py` becomes `models`. The package name must be a
canonical Morphir package path such as `acme/example`. An empty `exposedModules`
exposes this module; otherwise the list must name this module alone. All emitted
types and constructors are public. Declaration names use Morphir's existing
name codec, including uppercase initialism segments. Collisions after
normalization are errors. Names must be ASCII identifiers without a leading
underscore; Python keywords and the builtins used by this subset are reserved.
Module filenames also exclude Windows device names such as `con` and `aux`.

The backend returns one relative `.py` artifact. It rejects names that cannot
roundtrip through this spelling, including collisions between a type and a
constructor. The common Morphir pattern where a type and its single constructor
share a name therefore needs distinct names for this first Python subset.

## Current boundary

Supported fields are scalars, local references and fixed tuples of at least two
elements. The two imports shown above, frozen dataclasses, and non-generic
`type` aliases of dataclass variants are the only accepted module statements.
Comments and whitespace are not preserved. Functions, methods, field defaults,
inheritance, arbitrary decorators or imports, docstrings, generic parameters,
containers, optional fields, quoted annotations and multi-module compilation
are rejected. The backend also rejects private definitions, values, documentation,
attributes, dependencies and IR type forms outside this subset. Failures return
diagnostics with no partial IR or artifacts.

`typesOnly` does not enable skipping functions. This version only accepts types
regardless of that flag. `irVersion` accepts `4` or `4.0.0`. The CLI's
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

Semantic frontend diagnostics identify the source module. Syntax diagnostics
identify the parser's failing range. Source positions use the SDK's UTF-16
convention.

## Run the pipeline

From the `morphir-rust` checkout:

```console
cargo test --locked -p morphir-python-binding
cargo run --locked -p morphir-python-binding --example python_adt
cargo build --locked --release -p morphir-python-binding --target wasm32-unknown-unknown
cargo test --locked -p morphir-daemon --test python_extension -- --ignored
```

The example prints JSON containing the compiled IR and generated source. The
integration test invokes both capabilities through the actual WASM guest.
`MORPHIR_PYTHON_WASM` can select a different guest file for that test.
The native `NativeExtension::frontend_backend(PythonExtension)` adapter exposes
the same MEP methods without requiring a WASM runtime.

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
