---
layout: default
title: Gleam Language Binding Tutorial
nav_order: 2
parent: Tutorials
---

# Gleam Language Binding Tutorial

This tutorial covers using Morphir with Gleam, including compilation, code generation, and roundtrip testing.

## Overview

The Gleam binding provides:
- **Frontend**: Parse Gleam source → Morphir IR V4
- **Backend**: Morphir IR V4 → Generate Gleam source

## Setting Up a Gleam Project

### Option 1: New Gleam Project

If you're starting fresh:

```bash
mkdir my-gleam-project
cd my-gleam-project
mkdir src
```

Create `src/main.gleam`:

```gleam
pub fn main() {
  "Hello, Morphir!"
}
```

### Option 2: Existing Gleam Project

If you have an existing Gleam project, just add a `morphir.toml` file to the root.

## Configuration

Create `morphir.toml`:

```toml
[project]
name = "my-gleam-package"
version = "0.1.0"
source_directory = "src"

[frontend]
language = "gleam"
```

## Compiling Gleam to Morphir IR

### Basic Compilation

```bash
morphir gleam compile
```

This uses the configuration from `morphir.toml` to:
- Find source files in `src/`
- Compile to Morphir IR V4
- Output to `.morphir/out/my-gleam-package/compile/gleam/`

### Custom Input/Output

```bash
morphir gleam compile --input src/ --output custom-output/
```

## Generating Gleam Code from IR

### Basic Generation

```bash
morphir gleam generate
```

This reads from the default compile output and generates to `.morphir/out/my-gleam-package/generate/gleam/`.

## Roundtrip Testing

Roundtrip testing verifies that compiling and generating preserves semantics:

```bash
morphir gleam roundtrip --input src/
```

## JSON Output

For programmatic use:

```bash
morphir gleam compile --json
morphir gleam compile --json-lines
```

## Next Steps

- Learn about [Configuration](configuration-guide)
- See [Complete Workflow](complete-workflow)
