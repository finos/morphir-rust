---
layout: default
title: Python ADT extension
nav_order: 6
parent: Tutorials
---

# Python ADT extension

The `morphir-python-binding` crate provides the `morphir-python` frontend and
backend. Start with its [modeling example and supported subset](../../crates/morphir-python-binding/README.md).
The extension compiles frozen dataclasses, named unions, fixed tuples and annotated conditional
functions to Morphir IR v4, and generates Python 3.12 or later code from those definitions.

This checkout also supports multiple source modules and imports between them.
That addition requires a development bundle built from this checkout; the
published Python `0.1.0` bundle supports one module per request.

Functions support returning `if`/`elif`/`else` branches and Python conditional
expressions. For example:

```python
def choose(flag: bool, first: int, second: int) -> int:
    if flag:
        return first
    else:
        return second
```

This body maps to Morphir's `IfThenElse` value. Each path must return the declared
type, and the condition must be boolean. Compile functions with `typesOnly=false`
through MEP, or omit the CLI's `--types-only` flag. The crate README lists the
supported expressions and remaining limits.

## Build and test

Run these commands from the `morphir-rust` repository:

```console
rustup target add wasm32-unknown-unknown
cargo test --locked -p morphir-python-binding
cargo run --locked -p morphir-python-binding --example python_adt
cargo run --locked -p morphir-python-binding --example python_modules
cargo build --locked --release -p morphir-python-binding --target wasm32-unknown-unknown
cargo test --locked -p morphir-daemon --test python_extension -- --ignored --exact python_adt_and_conditional_roundtrip_through_the_real_wasm_extension
```

The example returns JSON containing IR and generated Python. The WASM test
performs capability negotiation, compilation, generation and recompilation
through the daemon's actual extension container. CI runs it with a release guest.

## Install a release bundle

The Python extension is released independently as `extension/python/v0.1.0` in
[finos/morphir-rust releases](https://github.com/finos/morphir-rust/releases).
It contains a WASM guest, SHA-256 checksum and release descriptor. It is a Morphir
extension, not a PyPI package or a CPython import module.

Use the Rust Morphir CLI with frontend-bundle publication support; CLI
`0.4.0-alpha.6` and earlier cannot publish this descriptor. The npm `morphir-elm`
executable does not provide these installation commands.

Download and install with PowerShell:

```powershell
gh release download extension/python/v0.1.0 --repo finos/morphir-rust --dir python-bundle
Rename-Item -LiteralPath python-bundle/morphir-python-binding-0.1.0.release.json -NewName release.json
$pythonIndex = Join-Path (Get-Location) '.morphir/python-index'
$env:MORPHIR_HOME = Join-Path (Get-Location) '.morphir/python-home'
morphir extension repository init $pythonIndex
morphir extension repository add python-local --directory $pythonIndex
morphir extension repository publish python-local --bundle python-bundle
morphir extension repository verify python-local
morphir extension install --repository python-local morphir-python
```

Keep the same `MORPHIR_HOME` for compilation and generation. Installation verifies
the artifact digest and records both capabilities. Once installed, compilation
and generation work offline without access to the original repository.

## Build a development bundle

From `morphir-rust`, run `mise run extension:artifact:python`. This builds and
validates the guest, packages it, and tests installation and offline roundtripping.
Use `.morphir/build/extensions/python` as the `--bundle` directory in the
publication command above; its descriptor is already named `release.json`.
CI also uploads this directory as the `morphir-python-extension-bundle` artifact.

For a Windows build using the native toolchain, after the build commands above:

```powershell
python -B scripts/package_extension.py --short-id python --wasm target/wasm32-unknown-unknown/release/morphir_python_binding.wasm --output .morphir/build/extensions/python
$env:MORPHIR_PYTHON_BUNDLE = (Resolve-Path .morphir/build/extensions/python).Path
cargo test --locked -p morphir-daemon --test python_extension -- --ignored
```

The release pipeline builds from a committed snapshot and verifies that the
bundle's `gitCommit` matches the release tag. Development bundles from modified
working trees omit that provenance claim.

## Compile and generate

Create a project directory with a `src/models.py` file using the example above,
and this `morphir.toml`:

```toml
[project]
name = "acme/example"
version = "0.1.0"
source_directory = "src"

[frontend]
language = "python"
emit_parse_stage = false

[codegen]
targets = ["python"]
```

From that project directory, use the Rust CLI's installed provider path:

```console
morphir compile --language python --input src/models.py --output compiled
morphir generate --target python --input compiled/morphir-ir.json --output generated
```

Compilation installs `morphir-ir.json` under the `compiled` directory. The
backend returns `models.py` as an artifact. The host writes it to the output
location according to the CLI's install/output settings. The extension itself
has no filesystem access. It fails with diagnostics if the request contains
unsupported imports, function behavior, or types outside the
documented subset. The [binding README](../../crates/morphir-python-binding/README.md)
lists the supported IR nodes, tuple forms, and the scope of its conformance evidence.

With the development bundle installed, place related files under `src/` and
compile the directory to resolve their imports together:

```console
morphir compile --language python --input src --output compiled
morphir generate --target python --input compiled/morphir-ir.json --output generated
morphir compile --language python --input generated --output recompiled
```

For example, `src/domain/models.py` may define `Decision`, and
`src/domain/rules.py` may use `from .models import Decision` in its annotations.
The resulting IR contains `domain/models` and `domain/rules`; generation returns
`domain/models.py` and `domain/rules.py`. Nested directories are namespace
packages, without `__init__.py` files. Keep all referenced modules in the source
set. Cross-package dependencies and imported function calls remain unsupported.

With a CLI and development bundle containing project-context support, the
configured source directory is sufficient:

```console
morphir compile --project packages/orders
morphir generate --project packages/orders --target python --output generated
```

The project selector accepts a declared workspace-relative path or exact
project name. Compilation uses that member's source directory and package name;
generation reads its recorded compile artifact. An explicit `--input` replaces
the configured input and resolves from the invocation directory.

Set `project.exposed_modules = ["domain.rules"]` to expose only that module.
`domain.models` remains in the IR as a private module and is still available to
sibling imports. Generation emits both files. Preserve the exposure configuration
when recompiling generated source: Python itself does not enforce Morphir module
privacy. Omitting exposure makes all modules public; `[]` makes all private.
