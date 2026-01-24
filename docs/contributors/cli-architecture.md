---
layout: default
title: CLI Architecture
nav_order: 3
parent: For Contributors
---

# CLI Architecture

This document describes the architecture of the Morphir CLI, including command structure, configuration handling, and extension integration.

## Command Structure

The CLI uses `clap` for argument parsing and `starbase` for application lifecycle:

```
morphir
├── compile          # Language-agnostic compilation
├── generate         # Language-agnostic code generation
├── gleam            # Gleam-specific commands
│   ├── compile
│   ├── generate
│   └── roundtrip
├── extension        # Extension management
├── ir               # IR operations
└── ...
```

## Command Execution Flow

### Compile Command

```
1. Parse CLI arguments
2. Discover configuration (morphir-design)
3. Load config context (workspace/project detection)
4. Merge CLI args with config (CLI overrides)
5. Resolve paths (.morphir/out/<project>/compile/<language>/)
6. Discover extension by language (morphir-design → morphir-daemon)
7. Load extension WASM (morphir-daemon)
8. Collect source files
9. Call extension.frontend.compile()
10. Write IR to output directory
11. Format output (human/JSON/JSON Lines)
```

### Generate Command

```
1. Parse CLI arguments
2. Discover configuration
3. Load config context
4. Resolve IR input path
5. Discover extension by target (morphir-design → morphir-daemon)
6. Load extension WASM
7. Load Morphir IR (detect format)
8. Call extension.backend.generate()
9. Write generated code to output directory
10. Format output
```

## Configuration Handling

### Discovery

Configuration is discovered by walking up the directory tree:

```rust
let config_path = discover_config(&current_dir)?;
let ctx = load_config_context(&config_path)?;
```

### Merging

Configuration is merged in priority order:
1. Workspace config (if in workspace)
2. Project config
3. CLI arguments (highest priority)

### Path Resolution

Paths are resolved relative to:
- Config file location (for relative paths)
- Workspace root (for workspace paths)
- Project root (for project paths)

## Extension Integration

### Discovery

```rust
// Design-time discovery
let builtins = morphir_design::discover_builtin_extensions();

// Daemon registry
let registry = ExtensionRegistry::new(workspace_root, output_dir)?;

// Register builtins
for builtin in builtins {
    registry.register_builtin(&builtin.id, builtin.path).await?;
}

// Find extension
let extension = registry.find_extension_by_language("gleam").await?;
```

### Execution

```rust
// Call extension method
let params = serde_json::json!({
    "input": input_path,
    "output": output_path,
    "package_name": package_name,
});

let result: serde_json::Value = extension
    .call("morphir.frontend.compile", params)
    .await?;
```

## Output Formatting

### Human-Readable

Default format with progress messages and diagnostics.

### JSON

Single JSON object with structured data:

```json
{
  "success": true,
  "ir": {...},
  "diagnostics": [...],
  "modules": [...]
}
```

### JSON Lines

Streaming format (one JSON object per line):

```jsonl
{"type": "progress", "message": "Compiling..."}
{"type": "result", "success": true, "ir": {...}}
```

## Error Handling

Errors are handled using `miette` for rich diagnostics:

- **Human mode**: Pretty-printed errors with source spans
- **JSON mode**: Structured error objects

## Next Steps

- See [Design-Time Crate](design-time-crate)
- Read [Extension System Design](extension-system)
- Check [Development Guide](development)
