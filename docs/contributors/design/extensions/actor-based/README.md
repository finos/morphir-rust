---
layout: default
title: Morphir Extension System Design Documents
nav_order: 1
parent: Morphir Extensions
has_children: true
---

# Morphir Extension System Design Documents

This directory contains the complete design documentation for the Morphir Extension System, an actor-based, multi-protocol architecture for extending Morphir functionality.

## Document Index

### Core Architecture

- **[00-overview.md](00-overview.md)** - System overview, goals, and key concepts
- **[01-architecture.md](01-architecture.md)** - Detailed architecture, components, and data flow
- **[02-extension-host-interface.md](02-extension-host-interface.md)** - Common interface for all extension hosts

### Protocol Implementations

- **[03-jsonrpc-host.md](03-jsonrpc-host.md)** - JSON-RPC 2.0 over HTTP
- **[04-grpc-host.md](04-grpc-host.md)** - gRPC with Protocol Buffers
- **[05-stdio-host.md](05-stdio-host.md)** - JSON Lines over stdin/stdout
- **[06-extism-wasm-host.md](06-extism-wasm-host.md)** - Extism WASM runtime
- **[07-wasm-component-host.md](07-wasm-component-host.md)** - WASM Component Model

### Integration & Operations

- **[08-extension-manager.md](08-extension-manager.md)** - Central coordinator actor
- **[09-security-and-isolation.md](09-security-and-isolation.md)** - Security model and sandboxing
- **[10-protocol-specifications.md](10-protocol-specifications.md)** - Wire protocol details
- **[11-examples-and-recipes.md](11-examples-and-recipes.md)** - Practical examples and patterns

## Quick Start

For a quick understanding of the system:

1. Read [00-overview.md](00-overview.md) for the big picture
2. Read [01-architecture.md](01-architecture.md) for component details
3. Choose your protocol (likely [05-stdio-host.md](05-stdio-host.md) for simplicity)
4. Follow examples in [11-examples-and-recipes.md](11-examples-and-recipes.md)

## Key Technologies

- **Actor Framework**: Kameo (Rust)
- **Protocols**: JSON-RPC 2.0, gRPC, Stdio (JSON Lines), WASM (Extism, Component Model)
- **Concurrency**: Tokio async runtime
- **Security**: Process isolation, WASM sandboxing, capability-based permissions

## Design Status

**Status**: Draft  
**Version**: 0.1.0  
**Last Updated**: 2025-01-23

These documents are design drafts intended for incorporation into the FINOS Morphir project documentation.

## Contributing

When implementing these designs:

1. Maintain actor-based isolation
2. Follow the `ExtensionHost` trait contract
3. Implement all protocol lifecycle methods
4. Add comprehensive tests
5. Update security documentation as needed

## Related Resources

- [FINOS Morphir Project](https://github.com/finos/morphir)
- [Kameo Actor Framework](https://github.com/tqwewe/kameo)
- [Extism WASM Framework](https://extism.org)
- [WASM Component Model](https://component-model.bytecodealliance.org)
