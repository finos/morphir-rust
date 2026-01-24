---
layout: default
title: Design Documents
nav_order: 1
parent: For Contributors
---

# Extension System Design Documents

This directory contains design documents for the Morphir extension system.

## Overview

The extension system design documents describe the architecture and vision for Morphir's extension system, including:

- Extension Manager architecture (actor-based coordination)
- Multi-protocol extension hosts (JSON-RPC, gRPC, stdio, Extism WASM, WASM Component)
- Extension discovery and loading
- Workspace lifecycle management

## Current Status

The current implementation uses a simplified approach compatible with the design vision:

- **Current**: Direct Extism integration for WASM extensions
- **Future**: Full actor model with Extension Manager and protocol hosts

## Design Documents

Design documents from `morphir-extension-system-design-docs.zip` should be extracted here:

- `00-overview.md` - System overview
- `01-architecture.md` - Architecture details
- `02-extension-host-interface.md` - ExtensionHost trait
- `06-extism-wasm-host.md` - Extism implementation
- `08-extension-manager.md` - Extension Manager design

## References

- [Extension System Design](../extension-system) - Current implementation overview
- [Architecture Overview](../architecture) - System architecture
- [CLI Architecture](../cli-architecture) - CLI integration
