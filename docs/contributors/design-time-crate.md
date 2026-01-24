---
layout: default
title: Design-Time Crate
nav_order: 4
parent: For Contributors
---

# Design-Time Crate

The `morphir-design` crate provides reusable design-time tooling that can be used by the CLI, IDEs, build tools, and other Morphir tooling.

## Purpose

The design-time crate separates concerns:
- **CLI**: User-facing commands and output formatting
- **Design-Time**: Configuration and extension discovery (reusable)
- **Common**: Shared data structures and utilities
- **Daemon**: Runtime extension execution

This allows IDEs and other tools to use design-time functionality without CLI dependencies.

## Key Functionality

### Configuration Discovery

```rust
// Walk up directory tree to find morphir.toml
let config_path = discover_config(&start_dir)?;

// Load config with workspace/project context
let ctx = load_config_context(&config_path)?;
```

### Extension Discovery

```rust
// Discover builtin extensions
let builtins = discover_builtin_extensions();

// Get builtin extension path
let path = get_builtin_extension_path("gleam")?;
```

### Path Resolution

```rust
// Resolve compile output path
let compile_path = resolve_compile_output(
    "My.Project",
    "gleam",
    &morphir_dir
);

// Resolve generate output path
let generate_path = resolve_generate_output(
    "My.Project",
    "gleam",
    &morphir_dir
);
```

## Usage in IDEs

IDEs can use the design-time crate to:
- Discover project configuration
- Resolve paths for build outputs
- Find available extensions
- Determine workspace/project context

Example:

```rust
use morphir_design::{discover_config, load_config_context};

// In IDE plugin
let ctx = load_config_context(&discover_config(&project_dir)?)?;
let compile_path = resolve_compile_output(
    &ctx.config.project.as_ref().unwrap().name,
    "gleam",
    &ctx.morphir_dir
);
```

## Usage in Build Tools

Build tools can integrate Morphir compilation:

```rust
use morphir_design::{discover_config, ensure_morphir_structure};

// In build script
let ctx = load_config_context(&discover_config(&project_dir)?)?;
ensure_morphir_structure(&ctx.morphir_dir)?;
// ... trigger compilation
```

## API Stability

The design-time crate API is designed to be stable and reusable. Changes should maintain backward compatibility when possible.

## Next Steps

- See [Architecture Overview](architecture)
- Read [CLI Architecture](cli-architecture)
- Check [Development Guide](development)
