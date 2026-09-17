---
layout: default
title: Python ADT extension
nav_order: 6
parent: Tutorials
---

# Python ADT extension

The `morphir-python-binding` crate provides the `morphir-python` frontend and
backend. Start with its [modeling example and supported subset](../../crates/morphir-python-binding/README.md).
The extension compiles frozen dataclasses, named unions and annotated conditional
functions to Morphir IR v4, and generates Python 3.12 or later code from those definitions.

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
cargo build --locked --release -p morphir-python-binding --target wasm32-unknown-unknown
cargo test --locked -p morphir-daemon --test python_extension -- --ignored
```

The example returns JSON containing IR and generated Python. The WASM test
performs capability negotiation, compilation, generation and recompilation
through the daemon's actual extension container. CI runs it with a release guest.

## Install a local development build

This extension is not yet published. The existing release-bundle packager is
backend-specific, so use a controlled local index that declares both capabilities.
The following PowerShell creates that index from the build above. Run it from
`morphir-rust`. If `CARGO_TARGET_DIR` is set, adjust the artifact path accordingly.

```powershell
$pythonIndex = Join-Path (Get-Location) '.morphir/python-index'
New-Item -ItemType Directory -Force "$pythonIndex/artifacts", "$pythonIndex/extensions" | Out-Null
$pythonGuest = 'target/wasm32-unknown-unknown/release/morphir_python_binding.wasm'
Copy-Item -LiteralPath $pythonGuest -Destination "$pythonIndex/artifacts/morphir_python_binding.wasm"
$pythonDigest = (Get-FileHash -Algorithm SHA256 -LiteralPath $pythonGuest).Hash.ToLowerInvariant()
$pythonMetadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$pythonVersion = ($pythonMetadata.packages | Where-Object name -eq 'morphir-python-binding').version
$pythonRelease = @{
    schemaVersion = '1.0'
    id = 'morphir-python'
    name = 'Morphir Python'
    version = $pythonVersion
    channels = @('stable')
    mepVersions = @('0.1')
    capabilities = @('frontend', 'backend')
    frontend = @{
        languages = @(@{ id = 'python'; fileExtensions = @('.py') })
        irVersions = @('4')
    }
    backend = @{ targets = @('python'); irVersions = @('4') }
    artifacts = @(@{
        runtime = 'wasm'
        source = @{ kind = 'local-file'; path = 'artifacts/morphir_python_binding.wasm' }
        sha256 = $pythonDigest
        filename = 'morphir_python_binding.wasm'
    })
}
[IO.File]::WriteAllText("$pythonIndex/extensions/morphir-python.jsonl",
    ($pythonRelease | ConvertTo-Json -Depth 10 -Compress) + "`n")
$env:MORPHIR_HOME = Join-Path (Get-Location) '.morphir/python-home'
morphir extension repository init $pythonIndex
morphir extension repository add python-local --directory $pythonIndex
morphir extension repository verify python-local
morphir extension install --repository python-local morphir-python
```

Here `morphir` must be the Rust CLI from `finos/morphir`, not the npm
`morphir-elm` executable. Keep the same `MORPHIR_HOME` for compilation and
generation. Installation checks the artifact digest and records both frontend
and backend capabilities.

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
more than one module, runtime behavior, or types outside the documented subset.
