---
layout: default
title: morphir generate
parent: CLI Reference
nav_order: 21
---

# `morphir generate`

**Usage**: `morphir generate [OPTIONS]`

Generate code from Morphir IR using target-specific extensions.

## Options

### `-t, --target <TARGET>`

Target language or format for code generation (e.g., `gleam`, `typescript`, `scala`). If not specified, uses the first target from `morphir.toml`.

### `-i, --input <INPUT>`

Path to Morphir IR file or directory. Defaults to the compile output for the target language.

### `-o, --output <OUTPUT>`

Output directory for generated code. Defaults to `.morphir/out/<project>/generate/<target>/`.

### `--config <PATH>`

Explicit path to morphir.toml or morphir.json configuration file.

### `--project <NAME>`

Project name (for workspaces with multiple projects).

### `--json`

Output results as JSON.

### `--json-lines`

Output results as JSON Lines (streaming format).

## Configuration

The generate command reads settings from `morphir.toml`:

```toml
[project]
name = "my-project"

[codegen]
targets = ["gleam", "typescript"]
```

## Examples

```bash
# Generate code using config defaults
morphir generate

# Generate TypeScript from IR
morphir generate --target typescript

# Generate from a specific IR file
morphir generate --target gleam --input ./morphir-ir.json

# Generate to a specific output directory
morphir generate --output ./generated
```

## Output Structure

Generated code is written to the Mill-inspired output structure:

```
.morphir/
  out/
    <project>/
      generate/
        <target>/
          <generated files>
```

## Workflow

The typical workflow is:

1. **Compile**: `morphir compile` - Convert source to IR
2. **Generate**: `morphir generate` - Convert IR to target code

```bash
# Full roundtrip
morphir compile --language gleam
morphir generate --target typescript
```

## See Also

- [morphir compile](compile) - Compile source code to IR
- [morphir gleam generate](gleam/generate) - Gleam-specific generate command
