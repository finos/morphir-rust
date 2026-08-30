---
layout: default
title: "Historical: Actor-Based Extension Design"
nav_exclude: true
search_exclude: true
has_toc: false
---

# Historical actor-based extension design

> **Historical design, not a supported extension contract.** This directory
> preserves a superseded actor-based, multi-protocol proposal. Current
> extensions use Extism plus MEP JSON-RPC and install by ID from a controlled
> index. See the [current extension guide](../README.md).

These pages retain design rationale about isolation, lifecycle, transport, and
capability boundaries. They do not describe the current CLI or extension ABI.
The gRPC, HTTP JSON-RPC, JSON Lines, Kameo actor, WIT Component Model, and
`morphir.toml` extension configuration examples were never released.

## Historical document set

- [Overview](00-overview.md)
- [Architecture](01-architecture.md)
- [Extension host interface](02-extension-host-interface.md)
- [JSON-RPC host](03-jsonrpc-host.md)
- [gRPC host](04-grpc-host.md)
- [Stdio host](05-stdio-host.md)
- [Extism WASM host](06-extism-wasm-host.md)
- [WASM Component Model host](07-wasm-component-host.md)
- [Extension manager](08-extension-manager.md)
- [Security and isolation](09-security-and-isolation.md)
- [Protocol specifications](10-protocol-specifications.md)
- [Examples and recipes](11-examples-and-recipes.md)
- [WASM architecture session](12-wasm-extension-architecture-session.md)

For current work, use the [extension system overview](../../../extension-system.md)
and the current guide linked above.
