# Steel Thread Integration Test Plan

**Version:** 1.0
**Date:** 2026-01-29
**Status:** Draft

## Overview

This document defines the integration testing strategy for the **Steel Thread**: end-to-end validation of the WASM
extension architecture using the `migrate` command.

## Steel Thread Components

```
CLI (morphir migrate)
  │
  ├─> morphir-builtins::MigrateExtension (native mode)
  │     │
  │     └─> execute_native(&Envelope)
  │           │
  │           └─> Returns Envelope
  │
  └─> [Future] DirectRuntime + ExtismRuntime (WASM mode)
        │
        └─> Loads migrate.wasm
              │
              └─> Calls via Extism
```

## Test Pyramid

```
                     ┌─────────────────┐
                     │  E2E CLI Tests  │  ← Integration tests
                     │  (Gherkin)      │     (This document)
                     └────────┬────────┘
                              │
                    ┌─────────┴──────────┐
                    │  Component Tests   │  ← Unit tests
                    │  (Rust + Gherkin)  │     (Per-crate)
                    └─────────┬──────────┘
                              │
                    ┌─────────┴─────────┐
                    │   Unit Tests      │  ← Function-level
                    │   (Rust)          │     (Per-module)
                    └───────────────────┘
```

## Integration Test Layers

### Layer 1: morphir-builtins (Native Execution)

**Location:** `crates/morphir-builtins/tests/`

**Tests:**

- ✅ Unit tests (per-module)
- ✅ Gherkin tests (envelope protocol validation)
- ✅ Integration with morphir-ext-core (Envelope)

**Coverage:**

- `MigrateExtension::execute_native()` with various inputs
- Classic → V4 conversion
- V4 → Classic conversion
- Same-format passthrough
- Error handling (invalid IR, bad version)
- Warning generation (missing dependencies)

**Example Gherkin:**

```gherkin
Feature: Migrate Extension Native Execution
  Background:
    Given I have a MigrateExtension instance

  Scenario: Migrate Classic IR to V4
    Given I have a Classic IR file "classic-ir.json"
    When I create a migrate request envelope with:
      | field          | value    |
      | target_version | v4       |
      | expanded       | false    |
    And I call execute_native with the envelope
    Then the result envelope should indicate success
    And the migrated IR should be in V4 format
    And there should be a warning about dropped dependencies

  Scenario: Handle invalid target version
    Given I have a Classic IR file "classic-ir.json"
    When I create a migrate request envelope with:
      | field          | value    |
      | target_version | invalid  |
      | expanded       | false    |
    And I call execute_native with the envelope
    Then the result envelope should indicate failure
    And the error message should mention "Invalid target version"
```

### Layer 2: morphir-ext (Runtime Infrastructure)

**Location:** `crates/morphir-ext/tests/`

**Tests:**

- ExtismRuntime with test WASM modules
- DirectRuntime wrapping ExtismRuntime
- ExtensionInstance state management

**Coverage:**

- Load WASM module via ExtismRuntime
- Call envelope through WASM boundary
- State persistence across calls (TEA model)
- Error propagation from WASM to host

**Example Gherkin:**

```gherkin
Feature: DirectRuntime with Migrate Extension
  Background:
    Given I have compiled morphir-builtins to WASM
    And I have an ExtismRuntime loaded with migrate.wasm

  Scenario: Execute migrate via DirectRuntime
    Given I have a DirectRuntime wrapping the ExtismRuntime
    And I have a Classic IR envelope
    When I execute "backend_generate" with the envelope
    Then the call should succeed
    And the result should be a valid V4 IR envelope

  Scenario: Handle WASM execution error
    Given I have a DirectRuntime wrapping the ExtismRuntime
    And I have a malformed IR envelope
    When I execute "backend_generate" with the envelope
    Then the call should fail with a clear error message
```

### Layer 3: CLI Integration (End-to-End)

**Location:** `crates/integration-tests/tests/features/steel-thread.feature`

**Tests:**

- Full CLI command execution
- File I/O (read input, write output)
- Error reporting to user
- JSON and non-JSON output modes

**Coverage:**

- `morphir migrate input.json output.json` (native mode)
- `morphir migrate --mode wasm input.json output.json` (WASM mode, future)
- Remote sources (URLs, git repos)
- Error cases (missing file, invalid IR, etc.)

**Example Gherkin:**

```gherkin
Feature: Steel Thread - Migrate Command Integration
  As a Morphir user
  I want to migrate IR between versions
  So that I can upgrade my IR format

  Background:
    Given the morphir CLI is built and available
    And I have a temporary test directory

  Scenario: Migrate Classic IR to V4 format
    Given I have a Classic IR file "morphir-ir.json" with:
      """
      {
        "formatVersion": 1,
        "distribution": [
          "Library",
          "Library",
          ["com", "example"],
          [],
          {"modules": {}}
        ]
      }
      """
    When I run "morphir migrate morphir-ir.json output.json --target v4"
    Then the command should succeed
    And the file "output.json" should exist
    And the file "output.json" should contain V4 format IR
    And the output should contain "Migration complete"

  Scenario: Migrate with invalid target version
    Given I have a Classic IR file "morphir-ir.json"
    When I run "morphir migrate morphir-ir.json output.json --target invalid"
    Then the command should fail
    And the error output should contain "Invalid target version"

  Scenario: Migrate with missing input file
    When I run "morphir migrate nonexistent.json output.json --target v4"
    Then the command should fail
    And the error output should contain "Failed to load input"

  Scenario: Migrate to stdout (no output file)
    Given I have a Classic IR file "morphir-ir.json"
    When I run "morphir migrate morphir-ir.json --target v4 --json"
    Then the command should succeed
    And stdout should contain valid V4 IR JSON
    And stderr should contain migration metadata

  Scenario: Migrate from remote source (GitHub)
    Given I have internet connectivity
    When I run "morphir migrate github://finos/morphir-examples/main/morphir-ir.json output.json --target v4"
    Then the command should succeed
    And the file "output.json" should exist
    And the file "output.json" should contain V4 format IR
```

## Test Organization

### Directory Structure

```
crates/
├─ morphir-builtins/
│  ├─ tests/
│  │  ├─ migrate_native.rs              # Unit tests
│  │  └─ features/
│  │     └─ migrate_native.feature      # Gherkin (native mode)
│  └─ ...
│
├─ morphir-ext/
│  ├─ tests/
│  │  ├─ direct_runtime.rs              # Unit tests
│  │  ├─ extism_runtime.rs              # Unit tests
│  │  └─ features/
│  │     ├─ direct_runtime.feature      # Gherkin
│  │     └─ extism_runtime.feature      # Gherkin
│  └─ ...
│
└─ integration-tests/
   ├─ tests/
   │  ├─ steel_thread.rs                # Test runner
   │  └─ features/
   │     ├─ steel-thread-native.feature # Native mode
   │     └─ steel-thread-wasm.feature   # WASM mode (future)
   └─ ...
```

### Test Data

**Location:** `crates/integration-tests/fixtures/steel-thread/`

```
fixtures/steel-thread/
├─ classic-simple.json         # Minimal Classic IR
├─ classic-with-deps.json      # Classic IR with dependencies
├─ v4-simple.json              # Minimal V4 IR
├─ v4-with-deps.json           # V4 IR with dependencies
├─ invalid-format.json         # Malformed IR
└─ README.md                   # Fixture documentation
```

## Test Execution Strategy

### Phase 0: Unit Tests (Per-Component)

Run during development:

```bash
# Test individual components
cargo test -p morphir-builtins
cargo test -p morphir-ext
cargo test -p morphir
```

### Phase 1: Component Integration (Gherkin)

Validate components work together:

```bash
# Run Gherkin tests for each component
cargo test -p morphir-builtins --test cucumber_tests
cargo test -p morphir-ext --test cucumber_tests
```

### Phase 2: CLI Integration (Steel Thread)

End-to-end validation:

```bash
# Build CLI first
cargo build --release

# Run steel thread integration tests
cargo test -p integration-tests --test steel_thread
```

### Phase 3: WASM Mode (Future)

Once DirectRuntime + ExtismRuntime implemented:

```bash
# Compile builtins to WASM
cargo build --target wasm32-unknown-unknown --release -p morphir-builtins

# Run WASM integration tests
cargo test -p integration-tests --test steel_thread -- --features wasm
```

## CI/CD Integration

### GitHub Actions Workflow

```yaml
name: Steel Thread Integration Tests

on: [push, pull_request]

jobs:
  steel-thread-native:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Build CLI
        run: cargo build --release

      - name: Run unit tests
        run: |
          cargo test -p morphir-builtins
          cargo test -p morphir-ext

      - name: Run component Gherkin tests
        run: |
          cargo test -p morphir-builtins --test cucumber_tests
          cargo test -p morphir-ext --test cucumber_tests

      - name: Run steel thread integration tests
        run: cargo test -p integration-tests --test steel_thread

  steel-thread-wasm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Install Rust + wasm32 target
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown

      - name: Build CLI
        run: cargo build --release

      - name: Build WASM builtins
        run: cargo build --target wasm32-unknown-unknown --release -p morphir-builtins

      - name: Run WASM integration tests
        run: cargo test -p integration-tests --test steel_thread -- --features wasm
```

## Success Criteria

### Minimum Viable Steel Thread (P0)

- [ ] All morphir-builtins unit tests pass
- [ ] All morphir-builtins Gherkin tests pass
- [ ] CLI can execute `morphir migrate` with native builtin
- [ ] Classic → V4 conversion works end-to-end
- [ ] V4 → Classic conversion works end-to-end
- [ ] Error handling works (invalid inputs, missing files)
- [ ] CI runs steel thread tests on every commit

### Extended Steel Thread (P1)

- [ ] All morphir-ext unit tests pass (ExtismRuntime, DirectRuntime)
- [ ] All morphir-ext Gherkin tests pass
- [ ] morphir-builtins compiled to WASM
- [ ] CLI can execute `morphir migrate --mode wasm`
- [ ] WASM path produces identical results to native path
- [ ] Performance comparison documented (native vs WASM)

## Performance Benchmarks

Include performance regression tests:

```rust
#[test]
fn benchmark_migrate_native() {
    let start = std::time::Instant::now();

    // Run migrate 100 times
    for _ in 0..100 {
        migrate_classic_to_v4(classic_ir.clone());
    }

    let duration = start.elapsed();
    let avg = duration / 100;

    // Assert reasonable performance (adjust threshold)
    assert!(avg < std::time::Duration::from_millis(200),
        "Migrate too slow: {}ms average", avg.as_millis());
}
```

## Monitoring and Reporting

### Test Reports

Generate HTML reports from Cucumber:

```bash
cargo test --test steel_thread -- --format json > report.json
# Convert to HTML with cucumber-html-reporter or similar
```

### Coverage Reports

```bash
# Install cargo-tarpaulin
cargo install cargo-tarpaulin

# Generate coverage for steel thread components
cargo tarpaulin \
  --packages morphir-builtins morphir-ext integration-tests \
  --out Html \
  --output-dir coverage/
```

## Maintenance

### Adding New Test Cases

1. **Add fixture data** to `fixtures/steel-thread/`
2. **Write Gherkin scenario** in appropriate `.feature` file
3. **Implement step** if needed (most should reuse existing steps)
4. **Run test** locally: `cargo test -p integration-tests --test steel_thread`
5. **Commit** with message: `test: Add steel thread test for [scenario]`

### Debugging Test Failures

1. **Run single scenario:**
   ```bash
   cargo test -p integration-tests --test steel_thread -- "Migrate Classic IR to V4"
   ```

2. **Enable debug logging:**
   ```bash
   RUST_LOG=debug cargo test -p integration-tests --test steel_thread
   ```

3. **Inspect test artifacts:**
    - Check `target/tmp/` for test workspace directories
    - Review CLI output in test failures

## References

- [Steel Thread Design](../design/extensions/actor-based/12-wasm-extension-architecture-session.md#steel-thread-migrate-command)
- [Cucumber-rs Documentation](https://cucumber-rs.github.io/cucumber/current/)
- [Existing CLI Tests](../../crates/integration-tests/tests/cli.rs)
- Steel Thread Issue: `morphir-rust-steel-thread`

## Amendments

None yet.
