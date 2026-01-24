---
layout: default
title: Getting Started Tutorial
nav_order: 1
parent: Tutorials
---

# Getting Started Tutorial

This tutorial will walk you through setting up your first Morphir project and using the CLI to compile and generate code.

## Prerequisites

- Morphir CLI installed (see [Installation Guide](../install))
- A terminal
- Basic familiarity with command-line tools

## Step 1: Create a Project

Create a new directory for your project:

```bash
mkdir my-morphir-project
cd my-morphir-project
```

## Step 2: Create Source Files

Create a `src` directory and add some Gleam source files:

```bash
mkdir src
```

Create `src/main.gleam`:

```gleam
pub fn hello() {
  "world"
}

pub fn add(x: Int, y: Int) -> Int {
  x + y
}
```

## Step 3: Create Configuration

Create a `morphir.toml` file in the project root:

```toml
[project]
name = "my-package"
version = "0.1.0"
source_directory = "src"

[frontend]
language = "gleam"
```

## Step 4: Compile to Morphir IR

Compile your Gleam source to Morphir IR:

```bash
morphir gleam compile
```

Or using the language-agnostic command:

```bash
morphir compile --language gleam --input src/
```

This will:
- Parse your Gleam source files
- Convert them to Morphir IR V4
- Write the IR to `.morphir/out/my-package/compile/gleam/`

## Step 5: Inspect the Output

The compiled IR is stored in a document tree structure:

```
.morphir/
└── out/
    └── my-package/
        └── compile/
            └── gleam/
                ├── format.json
                └── modules/
                    └── my-package/
                        └── main/
                            ├── module.json
                            ├── types/
                            └── values/
```

You can view the `format.json` file to see the IR structure.

## Step 6: Generate Code (Optional)

Generate code from the IR:

```bash
morphir gleam generate
```

This will:
- Read the IR from `.morphir/out/my-package/compile/gleam/`
- Generate Gleam source code
- Write it to `.morphir/out/my-package/generate/gleam/`

## Step 7: Roundtrip Testing

Test the complete pipeline:

```bash
morphir gleam roundtrip --input src/
```

This compiles your source to IR, then generates code back, allowing you to verify the roundtrip preserves semantics.

## Understanding the .morphir/ Folder Structure

Morphir uses a Mill-inspired folder structure:

- `.morphir/out/<project>/compile/<language>/` - Compiled IR output
- `.morphir/out/<project>/generate/<target>/` - Generated code output
- `.morphir/out/<project>/dist/` - Distribution files
- `.morphir/test/` - Test fixtures and scenarios (version controlled)
- `.morphir/logs/` - Log files
- `.morphir/cache/` - Cache files

The `out/` directory should be added to `.gitignore`:

```gitignore
.morphir/out/
.morphir/logs/
.morphir/cache/
```

## Next Steps

- Learn about [Gleam Language Binding](gleam-tutorial)
- Explore [Configuration Options](configuration-guide)
- See [Complete Workflow Examples](complete-workflow)
