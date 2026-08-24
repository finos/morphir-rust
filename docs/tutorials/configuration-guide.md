---
layout: default
title: Configuration Guide
nav_order: 3
parent: Tutorials
---

# Configuration Guide

Morphir uses `morphir.toml` or `morphir.yaml` (or the legacy `morphir.json`) for project and workspace configuration. Both serializations describe the same configuration model.

## Configuration Discovery

Morphir automatically discovers the project configuration by walking up the directory tree. In each directory it checks `morphir.toml`, `morphir.yaml`, `.morphir/morphir.toml`, and `.morphir/morphir.yaml`; if more than one exists, discovery fails with an ambiguity error that names every candidate. `morphir.yml` is never discovered implicitly, but you can pass it with `--config`.

## Project Configuration

```toml
[project]
name = "my-package"
version = "0.1.0"
source_directory = "src"
```

```yaml
project:
  name: my-package
  version: "0.1.0"
  source_directory: src
```

## Workspace Configuration

```toml
[workspace]
members = ["project-a", "project-b"]
default_member = "project-a"
```

## Configuration Merging

Morphir builds the effective configuration by merging these sources from lowest to highest precedence:

| Priority | Source | Location |
|----------|--------|----------|
| 0 | Built-in defaults | (compiled in) |
| 100 | System | `/etc/morphir/morphir.toml`, or `%PROGRAMDATA%\morphir\morphir.toml` on Windows |
| 200 | Global user | `<config-dir>/morphir/morphir.toml` or `~/.morphir/morphir.toml` |
| 300 | Project | `morphir.toml` or `.morphir/morphir.toml` |
| 350 | Workspace member | The selected member's project configuration |
| 400 | User override | `.morphir/morphir.user.toml` (workspace root, then member) |
| 600 | Environment | `MORPHIR_*` variables |

Every file location also accepts a `morphir.yaml` (or `morphir.user.yaml`) alternative; a location must contain only one of them.

Maps merge recursively, arrays replace earlier arrays, and a `null` never overrides a lower-precedence value.

Environment variables use `__` between nesting levels, for example `MORPHIR_IR__STRICT_MODE=true` sets `ir.strict_mode`.

## Inspecting Configuration

```sh
# Which sources were considered
morphir config path

# The effective configuration
morphir config show

# Machine-readable output
morphir config path --json
morphir config show --json
```

## Next Steps

- See [Complete Workflow](complete-workflow)
