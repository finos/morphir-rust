---
layout: default
title: Morphir Daemon
nav_order: 1
parent: Design Documents
has_children: true
---

# Morphir Daemon

The Morphir Daemon is a long-running service that manages workspaces, projects, builds, and provides IDE integration.

## Tracking

| Type              | References                                                                                                                                                                                                                                                                                                                                |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Beads**         | morphir-l75 (caching), morphir-n6b (analyzer), morphir-369 (SQLite VFS)                                                                                                                                                                                                                                                                   |
| **GitHub Issues** | [#392](https://github.com/finos/morphir/issues/392) (pipeline types), [#393](https://github.com/finos/morphir/issues/393) (diagnostics), [#394](https://github.com/finos/morphir/issues/394) (JSON output), [#400](https://github.com/finos/morphir/issues/400) (analyzer), [#401](https://github.com/finos/morphir/issues/401) (caching) |
| **Discussions**   | [#88](https://github.com/finos/morphir/discussions/88) (package manager)                                                                                                                                                                                                                                                                  |

## Overview

The daemon provides:

- **Workspace Management**: Multi-project development with shared dependencies
- **Build Orchestration**: Coordinated builds in dependency order
- **File Watching**: Automatic recompilation on source changes
- **IDE Integration**: Language server protocol support
- **Package Publishing**: Pack and publish to registries

## Documents

| Document                                  | Status | Description                                   |
| ----------------------------------------- | ------ | --------------------------------------------- |
| [Lifecycle](./lifecycle.md)               | Draft  | Workspace creation, opening, closing          |
| [Projects](./projects.md)                 | Draft  | Project management within a workspace         |
| [Dependencies](./dependencies.md)         | Draft  | Dependency resolution and caching             |
| [Build](./build.md)                       | Draft  | Build orchestration and diagnostics           |
| [Watching](./watching.md)                 | Draft  | File system watching for incremental builds   |
| [Packages](./packages.md)                 | Draft  | Package format, registry backends, publishing |
| [Configuration](./configuration.md)       | Draft  | morphir.toml system overview                  |
| [Workspace Config](./workspace-config.md) | Draft  | Multi-project workspace configuration         |
| [CLI Interaction](./cli-interaction.md)   | Draft  | CLI-daemon communication and lifecycle        |

## Architecture

```
workspace-root/
├── morphir.toml              # Workspace configuration
├── .morphir/                 # Workspace-level cache and state
│   ├── deps/                 # Resolved dependencies (shared)
│   └── cache/                # Build cache
├── packages/
│   ├── core/                 # Project: my-org/core
│   │   ├── morphir.toml
│   │   └── src/
│   ├── domain/               # Project: my-org/domain
│   │   ├── morphir.toml
│   │   └── src/
│   └── api/                  # Project: my-org/api
│       ├── morphir.toml
│       └── src/
```

## Key Concepts

### Workspace vs Project

| Concept       | Scope             | Configuration                             |
| ------------- | ----------------- | ----------------------------------------- |
| **Workspace** | Multiple projects | `morphir.toml` with `[workspace]` section |
| **Project**   | Single package    | `morphir.toml` with `[project]` section   |

Both use the same `morphir.toml` file format. The presence of `[workspace]` section enables workspace mode.

### Workspace States

| State          | Description                        |
| -------------- | ---------------------------------- |
| `closed`       | Workspace is not active            |
| `initializing` | Workspace is being loaded          |
| `open`         | Workspace is ready for operations  |
| `error`        | Workspace has unrecoverable errors |

### Project States

| State      | Description                               |
| ---------- | ----------------------------------------- |
| `unloaded` | Project metadata loaded, IR not compiled  |
| `loading`  | Project is being compiled                 |
| `ready`    | Project IR is loaded and valid            |
| `stale`    | Source files changed, needs recompilation |
| `error`    | Project has compilation errors            |

## JSON-RPC Protocol

Daemon operations are exposed via JSON-RPC for client communication:

```
workspace/create, workspace/open, workspace/close
workspace/addProject, workspace/removeProject, workspace/listProjects
workspace/buildAll, workspace/clean, workspace/watch
daemon/health, daemon/capabilities
```

See [CLI Interaction](./cli-interaction.md) for connection modes, transport options, and CLI-to-daemon communication details. See [IR v4](../ir/README.md) for full protocol and type specifications.

## CLI Commands

```bash
morphir workspace init          # Create new workspace
morphir workspace add <path>    # Add project to workspace
morphir workspace build         # Build all projects
morphir workspace watch         # Watch and rebuild on changes
morphir pack                    # Create distributable package
morphir publish                 # Publish to registry
```

## Design Principles

1. **Lazy Loading**: Projects are not compiled until explicitly needed
2. **Incremental**: Only recompile what changed
3. **Shared Resolution**: Dependencies resolved once at workspace level
4. **Isolation**: Project failures don't break the workspace
5. **Observable**: Rich state and diagnostic information

## Related

### Morphir Rust Design Documents

- **[Extensions](../extensions/)** - WASM components and task system
- **[Design Documents](../)** - All design documents

### Main Morphir Documentation

- [Morphir Documentation](https://morphir.finos.org) - Main Morphir documentation site
- [Morphir LLMs.txt](https://morphir.finos.org/llms.txt) - Machine-readable documentation index
- [Morphir IR v4 Design](https://morphir.finos.org/docs/design/draft/ir/) - IR v4 design documents
- [Morphir IR Specification](https://morphir.finos.org/docs/morphir-ir-specification/) - Complete IR specification
