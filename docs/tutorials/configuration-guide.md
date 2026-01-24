---
layout: default
title: Configuration Guide
nav_order: 3
parent: Tutorials
---

# Configuration Guide

Morphir uses `morphir.toml` (or `morphir.json`) for project and workspace configuration.

## Configuration Discovery

Morphir automatically discovers configuration files by walking up the directory tree.

## Project Configuration

```toml
[project]
name = "my-package"
version = "0.1.0"
source_directory = "src"
```

## Workspace Configuration

```toml
[workspace]
members = ["project-a", "project-b"]
default_member = "project-a"
```

## Configuration Merging

Configuration is merged in this order:
1. Workspace config
2. Project config
3. CLI arguments (highest priority)

## Next Steps

- See [Complete Workflow](complete-workflow)
