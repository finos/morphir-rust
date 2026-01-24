---
layout: default
title: Development Guide
nav_order: 5
parent: For Contributors
---

# Development Guide

This guide helps developers contribute to Morphir Rust.

## Setting Up Development Environment

### Prerequisites

- Rust toolchain (1.70+)
- `wasm32-unknown-unknown` target for extension development
- Git

### Clone and Build

```bash
git clone https://github.com/finos/morphir-rust.git
cd morphir-rust
cargo build
```

### Run Tests

```bash
# All tests
cargo test

# Specific crate
cargo test -p morphir

# BDD acceptance tests
cd crates/morphir-gleam-binding
cargo test --test acceptance
```

## Project Structure

```
morphir-rust/
├── crates/
│   ├── morphir/              # CLI
│   ├── morphir-design/       # Design-time tooling
│   ├── morphir-common/       # Shared infrastructure
│   ├── morphir-daemon/       # Runtime services
│   ├── morphir-gleam-binding/ # Gleam extension
│   └── ...
├── docs/                     # Documentation site
└── ...
```

## Code Organization

### CLI Commands

Commands are in `crates/morphir/src/commands/`:
- `compile.rs` - Compile command
- `generate.rs` - Generate command
- `gleam.rs` - Gleam subcommands

### Design-Time

Design-time functionality in `crates/morphir-design/`:
- `config.rs` - Configuration discovery
- `extensions.rs` - Extension discovery

### Common Utilities

Shared code in `crates/morphir-common/`:
- `config/model.rs` - Configuration models
- `pipeline/` - Pipeline framework
- `vfs/` - Virtual file system

## Adding a New Command

1. Create command module in `crates/morphir/src/commands/`
2. Add to `Commands` enum in `main.rs`
3. Wire up execution in `MorphirSession::execute()`
4. Add help text in `help.rs`
5. Add tests in `tests/cli_integration.rs`

## Adding a New Extension

1. Create extension crate
2. Implement `Extension`, `Frontend`, and/or `Backend` traits
3. Build as WASM: `cargo build --target wasm32-unknown-unknown`
4. Add to builtin extensions (if applicable)
5. Update extension discovery

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_feature() {
        // Test implementation
    }
}
```

### Integration Tests

Integration tests in `tests/` directory:

```rust
#[tokio::test]
async fn test_command() {
    // Test command execution
}
```

### BDD Tests

BDD tests use Cucumber/Gherkin:

```gherkin
Feature: My Feature
  Scenario: Test case
    Given setup
    When action
    Then assertion
```

## Code Style

- Follow Rust standard formatting (`cargo fmt`)
- Use `clippy` for linting (`cargo clippy`)
- Document public APIs
- Write tests for new features

## Contributing

1. Create a feature branch
2. Make changes
3. Add tests
4. Update documentation
5. Submit pull request

## Next Steps

- Read [Architecture Overview](architecture)
- See [Extension System Design](extension-system)
- Check [CLI Architecture](cli-architecture)
