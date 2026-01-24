---
layout: default
title: Morphir Extension System - Overview
nav_order: 2
parent: Morphir Extensions
---

# Morphir Extension System - Overview

**Status:** Draft  
**Version:** 0.1.0  
**Last Updated:** 2025-01-23

## Purpose

The Morphir Extension System provides a pluggable architecture for extending Morphir's capabilities through multiple protocols and runtime environments. This enables developers to write extensions in their language of choice while maintaining type safety, security, and performance.

## Goals

1. **Multi-Protocol Support**: Enable extensions via JSON-RPC, gRPC, stdio, and WASM
2. **Language Agnostic**: Allow extensions in any language (TypeScript, Python, Go, Gleam, Rust, etc.)
3. **Type Safety**: Maintain strong typing at protocol boundaries
4. **Isolation**: Ensure extension failures don't crash the core system
5. **Performance**: Support both high-throughput (gRPC) and lightweight (stdio) options
6. **Security**: Sandbox untrusted extensions (WASM)
7. **Developer Experience**: Make extension development simple and well-documented

## Non-Goals

1. Dynamic code loading of unsafe native code
2. Backwards compatibility with pre-1.0 Morphir APIs (clean break)
3. Supporting every possible IPC mechanism (focus on proven protocols)

## Key Concepts

### Extension

A piece of code that extends Morphir functionality. Extensions can:

- Transform IR (custom backends, optimizers)
- Validate IR (custom linting, business rules)
- Generate code (new target languages)
- Provide tooling (formatters, analyzers)
- Integrate external services (databases, APIs)

### Extension Host

An actor that manages extensions using a specific protocol. Each host:

- Spawns/manages extension processes or runtimes
- Translates between Morphir's internal API and the protocol
- Handles failures and restarts
- Reports health and metrics

### Extension Manager

The central coordinator that:

- Routes requests to appropriate hosts
- Maintains extension registry
- Handles extension lifecycle
- Provides unified API to Morphir core

## Supported Protocols

| Protocol               | Use Case                    | Languages               | Performance | Isolation    |
| ---------------------- | --------------------------- | ----------------------- | ----------- | ------------ |
| **JSON-RPC 2.0**       | Web services, microservices | Any with HTTP           | Medium      | Process      |
| **gRPC**               | High-performance, typed     | Any with protobuf       | High        | Process      |
| **Stdio (JSON Lines)** | Simple tools, scripts       | Any                     | Low-Medium  | Process      |
| **Extism WASM**        | Sandboxed, portable         | Many via Extism PDK     | Medium      | WASM sandbox |
| **WASM Component**     | Future-proof, portable      | Rust, C, others growing | Medium-High | WASM sandbox |

## Architecture Principles

1. **Actor-Based Isolation**: Each extension host runs as an independent Kameo actor
2. **Protocol Abstraction**: Common `ExtensionHost` trait hides protocol details
3. **Fail-Safe**: Extension failures are isolated and recoverable
4. **Observable**: All extension operations emit metrics and logs
5. **Configurable**: Extensions declared in configuration files

## Document Organization

- **01-architecture.md**: Overall system architecture and component relationships
- **02-extension-host-interface.md**: Common trait and interfaces
- **03-08**: Individual host implementations
- **09**: Security model and sandboxing
- **10**: Wire protocol specifications
- **11**: Usage examples and recipes

## Related Documents

- [Morphir IR Specification](../ir-spec.md)
- [Morphir CLI Architecture](../cli-architecture.md)
- [Configuration Schema](../config-schema.md)
