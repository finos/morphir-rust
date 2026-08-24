---
layout: default
title: Devkit Crate
nav_order: 4
parent: For Contributors
---

# Devkit Crate

The `morphir-devkit` crate provides workspace, configuration, and extension discovery for tools that work on Morphir projects: the CLI, IDEs, build tools, and other Morphir tooling.

It is distinct from `morphir-extension-sdk`, which is the SDK for *building* extensions. `morphir-devkit` is for tools that *use* a Morphir workspace.

## Purpose

The devkit separates concerns:
- **CLI**: User-facing commands and output formatting
- **Devkit**: Configuration and extension discovery (reusable)
- **Common**: Shared data structures and utilities
- **Daemon**: Runtime extension execution

This allows IDEs and other tools to use workspace functionality without CLI dependencies.

## Key Functionality

### Configuration Discovery and Loading

```rust
use morphir_devkit::{discover_config, load_config_context};

// Walk up the directory tree to find morphir.toml or morphir.yaml
let config_path = discover_config(&start_dir)?.expect("no configuration found");

// Merge every configuration source and resolve workspace/project context
let ctx = load_config_context(&config_path)?;
```

`load_config_context` merges built-in defaults, system, global user, project, workspace member, user override, and `MORPHIR_*` environment sources in precedence order. `ctx.sources` reports which sources were consulted.

### Extension Discovery

```rust
use morphir_devkit::{discover_builtin_extensions, get_builtin_extension_path};

let builtins = discover_builtin_extensions();
let path = get_builtin_extension_path("gleam")?;
```

### Path Resolution

```rust
use morphir_devkit::{resolve_compile_output, resolve_generate_output};

let compile_path = resolve_compile_output("My.Project", "gleam", &morphir_dir);
let generate_path = resolve_generate_output("My.Project", "gleam", &morphir_dir);
```

## Usage in IDEs

IDEs can use the devkit to discover project configuration, resolve build output paths, find available extensions, and determine workspace/project context.

## Usage in Build Tools

```rust
use morphir_devkit::{discover_config, ensure_morphir_structure, load_config_context};

let ctx = load_config_context(&discover_config(&project_dir)?.expect("configuration"))?;
ensure_morphir_structure(&ctx.morphir_dir)?;
// ... trigger compilation
```

## API Stability

The devkit API is designed to be stable and reusable. Changes should maintain backward compatibility when possible.

## Next Steps

- See [Architecture Overview](architecture)
- Read [CLI Architecture](cli-architecture)
- Check [Development Guide](development)
