---
layout: default
title: Devkit Crate
nav_order: 4
parent: For Contributors
redirect_from:
  - /contributors/design-time-crate/
  - /contributors/design-time-crate.html
---

# Devkit crate

The `morphir-devkit` crate provides workspace discovery, configuration loading, and path conventions for tools that work on Morphir projects. The CLI, IDEs, and build tools can use it without depending on one another.

It is distinct from `morphir-extension-sdk`, which defines the contracts for building extensions, and `morphir-host`, which owns extension registration, resolution, and execution.

## Responsibilities

The crates divide the work as follows:

- The CLI handles commands, output formatting, and the set of built-in extensions linked into that executable.
- The devkit discovers workspace and project configuration, computes effective configuration, and resolves conventional paths.
- The extension SDK defines MEP data types and native capability traits.
- `morphir-host` provides the transport-neutral provider registry and MEP sessions, and `morphir-host-native` the process, Extism and native channels that reach a guest.
- The distribution crate resolves, verifies, installs, and activates process and WebAssembly artifacts.

The devkit does not scan beside an executable for built-in extension files. A host application registers its linked built-ins explicitly.

## Configuration discovery and loading

```rust
use morphir_devkit::{discover_config, load_config_context};

let config_path = discover_config(&start_dir)?.expect("no configuration found");
let context = load_config_context(&config_path)?;
```

`load_config_context` merges built-in defaults, system configuration, global user configuration, project configuration, workspace-member configuration, user overrides, and `MORPHIR_*` environment sources. `context.sources` records which sources the loader consulted.

## Discovery from a host-owned directory

A connected host that already owns a directory capability can capture discovery
without reopening a root path for configuration reads:

```rust
use cap_std::fs::Dir;
use morphir_devkit::{ConfigLoadOptions, WorkspaceDiscoveryRoot};
use std::path::Path;

fn capture_owned(directory: Dir, label: &Path) -> anyhow::Result<()> {
    let root = WorkspaceDiscoveryRoot::from_directory(directory, label)?;
    let captured = root.capture(&ConfigLoadOptions::project_only())?;
    let request_for_guest = captured.request().clone();
    let details = captured.into_typed_details()?;
    // Send request_for_guest to the installed discovery guest. Keep that guest's
    // actual response as membership authority; details supplies local configs.
    Ok(())
}
```

Construction and every capture check that the host-selected labels identify the
supplied directory before reading configuration. Renaming or replacing the root
or retargeting its label refuses capture. Development traversal uses the owned
`Dir`; the labels support diagnostics and confined absolute aliases.

The request owns the captured configuration text, mounts and environment. Typed
details decode from that exact request without reading the live filesystem or
process environment again. Capturing is synchronous and does not freeze concurrent
filesystem changes. Hosts still own worker permits, cancellation, later currentness
checks and the choice of which installed guest result to accept.

Traversal, alias and payload budgets are shared with the existing path APIs.
`project_only()` omits system/global configuration, personal overrides and process
environment. The default options retain those existing selections; a directory
capability does not remove or expand the other source grants in the options.

## Path resolution

Every task writes under one workspace-level out root. `resolve_out_root` finds that root, and `TaskPaths` gives one task its scratch directory and its result record inside it.

```rust
use morphir_devkit::{TaskId, TaskPaths, module_path, resolve_out_root};

let root = resolve_out_root(out_dir_flag, env_out_dir.as_deref(), Some(&context), &cwd);
let module = module_path(&context);

let compile = TaskPaths::new(&root, &module, &TaskId::compile())?;
let generate = TaskPaths::new(&root, &module, &TaskId::generate("scala"))?;
```

`TaskPaths::new` returns a `Result` because it checks the module path: `root.join(module_path)` is what places a task, so an absolute module path, or one holding `..`, would put the task's scratch directory outside the out root. `module_path` always returns a path that passes; the check catches a hand-built one.

For a member at `packages/orders` in a workspace rooted at `<ws>`, `compile.dest` is `<ws>/.morphir/out/packages/orders/compile.dest` and `compile.result` is the record beside it, `compile.json`. A standalone project is the root module, so its paths sit directly under the out root.

A member is identified by the path it is declared at, relative to the workspace root, not by where that directory really sits. The declared spelling still has to be a plain relative path — no absolute path, no `..`, no backslash — but the directory it names may be a symbolic link leading anywhere, including outside the workspace. Morphir reads that member's sources from wherever the link leads, warns once that it has done so, and still writes the member's output under `<ws>/.morphir/out/<declared path>/`. Confinement is a rule about what Morphir writes, not about where a declared member reads from.

The root is chosen by the `--out-dir` flag, then `MORPHIR_OUT_DIR`, then `[workspace].out_dir` in the workspace configuration, then `.morphir/out` under the workspace root. `resolve_out_root` is pure: the caller reads the flag and the environment and passes them in.

These helpers apply Morphir's output layout. They do not choose or activate an extension provider.

## Extension boundary

The provider registry in `morphir-host` resolves providers by requested capability and Morphir IR version. It filters ineligible providers before considering origin. If an installed provider and a built-in provider both match, the installed provider wins.

Provider origin remains separate from invocation mode. A native built-in can run as `NativeDirect` under `PreferDirect` or as `NativeMep` under `ProtocolOnly`. Installed providers run as `ProcessMep` or `WasmMep` under either policy.

The Morphir CLI owns the registration of the built-ins it links. `morphir-host` stays language-neutral and does not depend on a language extension. The CLI does not build Gleam in: Gleam is an extension released from [finos/morphir-gleam](https://github.com/finos/morphir-gleam).

## Use in other tools

IDEs and build tools can reuse configuration and workspace discovery from the devkit. A tool that executes extensions should create its own registry, register the built-ins it ships, add installed snapshots from the distribution crate, and resolve the requested capability through the `morphir-host` registry.

## Further reading

- [Architecture overview](architecture)
- [CLI architecture](cli-architecture)
- [Development guide](development)
- [ADR-0003: Dual native invocation for built-in extensions](adr/0003-dual-native-builtin-invocation)
