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

It is distinct from `morphir-extension-sdk`, which defines the contracts for building extensions, and `morphir-daemon`, which owns extension registration, resolution, and execution.

## Responsibilities

The crates divide the work as follows:

- The CLI handles commands, output formatting, and the set of built-in extensions linked into that executable.
- The devkit discovers workspace and project configuration, computes effective configuration, and resolves conventional paths.
- The extension SDK defines MEP data types and native capability traits.
- The daemon provides the transport-neutral provider registry and MEP sessions.
- The distribution crate resolves, verifies, installs, and activates process and WebAssembly artifacts.

The devkit does not scan beside an executable for built-in extension files. A host application registers its linked built-ins explicitly.

## Configuration discovery and loading

```rust
use morphir_devkit::{discover_config, load_config_context};

let config_path = discover_config(&start_dir)?.expect("no configuration found");
let context = load_config_context(&config_path)?;
```

`load_config_context` merges built-in defaults, system configuration, global user configuration, project configuration, workspace-member configuration, user overrides, and `MORPHIR_*` environment sources. `context.sources` records which sources the loader consulted.

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

The root is chosen by the `--out-dir` flag, then `MORPHIR_OUT_DIR`, then `[workspace].out_dir` in the workspace configuration, then `.morphir/out` under the workspace root. `resolve_out_root` is pure: the caller reads the flag and the environment and passes them in.

These helpers apply Morphir's output layout. They do not choose or activate an extension provider.

## Extension boundary

The provider registry in the daemon resolves providers by requested capability and Morphir IR version. It filters ineligible providers before considering origin. If an installed provider and a built-in provider both match, the installed provider wins.

Provider origin remains separate from invocation mode. A native built-in can run as `NativeDirect` under `PreferDirect` or as `NativeMep` under `ProtocolOnly`. Installed providers run as `ProcessMep` or `WasmMep` under either policy.

The Morphir CLI owns the built-in Gleam registration. `morphir-daemon` stays language-neutral and does not depend on the Gleam extension.

## Use in other tools

IDEs and build tools can reuse configuration and workspace discovery from the devkit. A tool that executes extensions should create its own registry, register the built-ins it ships, add installed snapshots from the distribution crate, and resolve the requested capability through the daemon.

## Further reading

- [Architecture overview](architecture)
- [CLI architecture](cli-architecture)
- [Development guide](development)
- [ADR-0003: Dual native invocation for built-in extensions](adr/0003-dual-native-builtin-invocation)
