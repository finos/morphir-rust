---
layout: default
title: Design Documents
nav_order: 5
parent: For Contributors
has_children: true
---

# Morphir Design Documents

This directory contains design documents for the Morphir daemon and extension system, covering architecture, protocols, and implementation details.

## Overview

This directory is organized into two main areas:

1. **Daemon Design** - Workspace management, build orchestration, file watching, and CLI-daemon interaction
2. **Extension Design** - WASM component model, extension interfaces, and task system

## Daemon Design Documents

The daemon design documents describe the Morphir daemon architecture for managing workspaces, projects, builds, and IDE integration:

### Core Documents

- **[README](daemon/README.md)** - Daemon overview and architecture
- **[Lifecycle](daemon/lifecycle.md)** - Workspace creation, opening, closing
- **[Projects](daemon/projects.md)** - Project management within a workspace
- **[Dependencies](daemon/dependencies.md)** - Dependency resolution and caching
- **[Build](daemon/build.md)** - Build orchestration and diagnostics
- **[Watching](daemon/watching.md)** - File system watching for incremental builds
- **[Packages](daemon/packages.md)** - Package format, registry backends, publishing
- **[Configuration](daemon/configuration.md)** - morphir.toml system overview
- **[Workspace Config](daemon/workspace-config.md)** - Multi-project workspace configuration
- **[CLI Interaction](daemon/cli-interaction.md)** - CLI-daemon communication and lifecycle

### Configuration Documents

- **[morphir.toml](daemon/morphir-toml.md)** - Complete configuration file specification
- **[Merge Rules](daemon/merge-rules.md)** - Configuration inheritance and merge behavior
- **[Environment](daemon/environment.md)** - Environment variables and runtime overrides

## Extension Design Documents

The extension design documents describe the architecture for adding capabilities to Morphir via WASM components and the task system:

### Core Documents

- **[README](extensions/README.md)** - Extension system overview and getting started
- **[WASM Components](extensions/wasm-component.md)** - Component model integration and WIT interfaces
- **[Tasks](extensions/tasks.md)** - Task system, dependencies, and hooks

## Key Concepts

### Daemon

The Morphir daemon is a long-running service that:

- Manages workspaces and projects
- Orchestrates builds in dependency order
- Watches files for automatic recompilation
- Provides IDE integration via JSON-RPC
- Hosts extensions for design-time and runtime use

### Extensions

Morphir extensions enable:

- Custom code generators (new backend targets)
- Custom frontends (new source languages)
- Additional tasks (build automation)
- Protocol integration (JSON-RPC based communication)

Extensions are implemented as:

- **WASM Components** - Sandboxed WebAssembly components using the Component Model
- **Native Executables** - JSON-RPC over stdio executables
- **Packages** - Distributable bundles with manifest

## Design Status

All documents in this directory are marked as **draft** status and represent the evolving design for the Morphir daemon and extension system. These documents guide implementation work in the morphir-rust repository.

## References

### Morphir Rust Documentation

- [Extension System Design](../extension-system) - Current implementation overview
- [Architecture Overview](../architecture) - System architecture
- [CLI Architecture](../cli-architecture) - CLI integration

### Main Morphir Documentation

- [Morphir Documentation](https://morphir.finos.org) - Main Morphir documentation site
- [Morphir LLMs.txt](https://morphir.finos.org/llms.txt) - Machine-readable documentation index for LLMs
- [Morphir Design Documents](https://github.com/finos/morphir/tree/main/docs/design/draft) - Source repository for these design documents
- [Morphir IR Specification](https://morphir.finos.org/docs/morphir-ir-specification/) - Complete IR specification
- [Morphir IR v4 Design](https://morphir.finos.org/docs/design/draft/ir/) - IR v4 design documents

### Related Design Documents

- [Daemon Design (Main Repo)](https://morphir.finos.org/docs/design/draft/daemon/) - Original daemon design documents
- [Extension Design (Main Repo)](https://morphir.finos.org/docs/design/draft/extensions/) - Original extension design documents
