---
layout: default
title: Architecture Overview
nav_order: 1
parent: For Contributors
---

# Architecture Overview

This document provides an overview of the Morphir Rust architecture.

## System Architecture

The system consists of:
- **CLI (morphir)**: User-facing commands
- **Design-Time (morphir-design)**: Configuration and extension discovery
- **Common (morphir-common)**: Shared infrastructure
- **Daemon (morphir-daemon)**: Runtime extension execution
- **Extensions**: Language-specific implementations

## Crate Responsibilities

Each crate has clear responsibilities for separation of concerns.

## Next Steps

- Read [Extension System Design](extension-system)
- See [CLI Architecture](cli-architecture)
