---
layout: default
title: morphir compile
parent: CLI Reference
nav_order: 20
---

# `morphir compile`

**Usage**: `morphir compile [OPTIONS]`

Compile source code to Morphir IR using language-specific extensions.

## Options

### `-l, --language <LANGUAGE>`

Source language to compile (e.g., `gleam`, `elm`). If not specified, uses the language from `morphir.toml`.

### `-i, --input <INPUT>`

Input source directory or file. If not specified, uses `source_directory` from config (resolved relative to the config file location).

### `-o, --output <OUTPUT>`

Output directory for compiled IR. Defaults to `.morphir/out/<project>/compile/<language>/`.

### `--package-name <NAME>`

Override the package name from config.

### `--config <PATH>`

Explicit path to morphir.toml or morphir.json configuration file.

### `--project <NAME>`

Project name (for workspaces with multiple projects).

### `--json`

Output results as JSON.

### `--json-lines`

Output results as JSON Lines (streaming format).

## Configuration

The compile command reads settings from `morphir.toml`:

```toml
[project]
name = "my-project"
source_directory = "src"

[frontend]
language = "gleam"
emit_parse_stage = true  # Write parse stage JSON to .morphir/out
```

## Examples

```bash
# Compile Gleam sources using config defaults
morphir compile

# Compile with explicit language and input
morphir compile --language gleam --input ./src

# Compile to a specific output directory
morphir compile --output ./build/ir

# Output as JSON for scripting
morphir compile --json
```

## Path Resolution

- **CLI `--input`**: Resolved relative to the current working directory
- **Config `source_directory`**: Resolved relative to the morphir.toml location

This allows running `morphir compile` from any directory in a workspace while still finding the correct source files.

## Output Structure

Compiled IR is written to the Mill-inspired output structure:

```
.morphir/
  out/
    <project>/
      compile/
        <language>/
          format.json          # IR format metadata
          pkg/
            <package>/
              <module>/
                types/
                  <type>.type.json
                values/
                  <value>.value.json
```

## See Also

- [morphir generate](generate) - Generate code from compiled IR
- [morphir gleam compile](gleam/compile) - Gleam-specific compile command
