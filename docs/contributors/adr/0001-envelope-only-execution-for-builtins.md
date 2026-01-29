# ADR-0001: Envelope-Only Execution for Builtin Extensions

**Status:** Accepted
**Date:** 2026-01-29
**Deciders:** Damian, Claude (design discussion)
**Tags:** extension-architecture, performance, builtins

## Context

Morphir's extension architecture uses an **envelope protocol** as the universal message format between hosts and
extensions. The envelope wraps all data with metadata (`header`, `content_type`, `content`) and enables host-agnostic,
language-neutral communication.

For **builtin extensions** (like `migrate`) that are implemented in native Rust and bundled with Morphir CLI, we have
two potential execution paths:

### Path A: Envelope-Only (Uniform Protocol)

```
CLI (Rust types)
  │
  ├─> Serialize to Envelope (JSON)
  │
  └─> BuiltinExtension::execute_native(&Envelope)
        │
        └─> Deserialize from Envelope
              │
              └─> Process (Rust types)
                    │
                    └─> Serialize result to Envelope
                          │
                          └─> Return Envelope
                                │
                                └─> Deserialize to Rust types

Cost: 4 serialization operations per call
```

### Path B: Native Fast Path (Dual API)

```
CLI (Rust types)
  │
  └─> BuiltinExtension::execute_direct(&RustType)
        │
        └─> Process (Rust types)
              │
              └─> Return Rust types

Cost: 0 serialization operations
```

### Performance Considerations

For a typical migrate operation with ~100KB IR:

- **Serialization overhead**: ~6-12ms total (4 ser/deser operations)
- **Actual processing**: ~50-200ms (IR parsing and transformation)
- **Overhead percentage**: ~3-6% of total operation time

## Decision

We will use **Path A: Envelope-Only execution** for all builtin extensions, including native Rust implementations.

**No native fast path** will be provided initially. All execution flows through the envelope protocol, even when both
caller and extension are native Rust code.

## Rationale

### 1. Architectural Simplicity and Uniformity

**Consistency across execution modes:**

- Native → Native builtin: Uses envelopes
- Native → WASM extension: Uses envelopes
- Daemon → Native builtin: Uses envelopes
- Daemon → WASM extension: Uses envelopes

One protocol, one code path, one mental model.

**Benefits:**

- Easier to explain and document
- Simpler testing (one execution path)
- Fewer surprises for extension authors
- Native and WASM truly identical from caller perspective

### 2. Negligible Performance Impact

**The overhead is acceptable:**

- 3-6% overhead for design-time operations (migrate, compile, generate)
- Not in performance-critical hot paths
- Operations are already I/O bound (disk, network) or CPU bound (IR processing)
- serde_json is highly optimized (~1-2GB/s throughput)

**Migrate example:**

```
Total operation time: 100ms
  ├─ Serialization overhead: 6ms (6%)
  └─ IR processing: 94ms (94%)
```

For one-time operations like migrate, 6ms is imperceptible.

### 3. Future-Proof Design

**When we add remote daemon:**

- Everything already uses envelopes
- No bifurcation between local/remote execution
- Clients transparently switch between DirectRuntime and DaemonClient

**When extensions call other extensions:**

- Uniform protocol enables composition
- No special cases for native-to-native

**When we add non-Rust extensions:**

- Envelope protocol is language-agnostic
- Python, JavaScript, Gleam extensions all use same protocol

### 4. Avoids Premature Optimization

**We don't have real performance data yet:**

- No profiling showing serialization is a bottleneck
- No user complaints about performance
- YAGNI (You Aren't Gonna Need It) applies

**If needed later:**

- Optimize specific hot paths with profiling data
- Add fast path only where it matters (e.g., tight evaluation loops)
- Don't optimize everything preemptively

### 5. Reduces Maintenance Burden

**Single API surface:**

- One execution path to test
- One code path to maintain
- Fewer opportunities for bugs

**Dual API complexity avoided:**

- No need to keep two implementations in sync
- No cognitive overhead choosing which API to use
- No documentation explaining when to use which path

### 6. Steel Thread Validates Real Architecture

**The steel thread should test what we'll deploy:**

- If we bypass envelopes with a fast path, we're not testing the real architecture
- Better to validate envelope protocol works end-to-end
- Proves the architecture is sound before expanding

## Consequences

### Positive

- ✅ Clean, uniform architecture
- ✅ Simpler implementation and testing
- ✅ One mental model for all execution modes
- ✅ Future-proof for daemon, composition, multi-language
- ✅ Steel thread validates actual deployed architecture

### Negative

- ❌ 3-6% serialization overhead for native-to-native calls
- ❌ Extra JSON ser/deser in debugger (harder to inspect than structs)
- ❌ Philosophical discomfort with "unnecessary" serialization

### Neutral

- ⚠️ Can revisit if profiling shows >10% time in serialization
- ⚠️ Fast path can be added later for specific hot extensions
- ⚠️ Trade-off favors simplicity over micro-optimization

## When to Revisit This Decision

Add a native fast path **only if** profiling shows:

1. **>10% of operation time spent in serialization** (not just 3-6%)
2. **Performance-critical hot path** (tight loops, not one-time operations)
3. **User-facing latency issue** (e.g., IDE autocomplete lag)

Then consider:

- Adding `execute_direct()` method for specific hot extensions only
- Keeping envelope path as the default and fallback
- Documenting when to use which path

## Implementation Details

### Builtin Extension Trait

```rust
pub trait BuiltinExtension: Send + Sync {
    /// Execute via envelope protocol (always available).
    fn execute_native(&self, input: &Envelope) -> Result<Envelope>;

    /// Get extension metadata.
    fn info(&self) -> BuiltinInfo;

    /// Get embedded WASM bytes (optional).
    #[cfg(feature = "wasm")]
    fn wasm_bytes() -> Option<&'static [u8]> {
        None
    }
}
```

**No** `execute_direct()` or typed methods in the trait.

### Migrate Extension Example

```rust
impl BuiltinExtension for MigrateExtension {
    fn execute_native(&self, input: &Envelope) -> Result<Envelope> {
        // Always go through envelope protocol
        let request: MigrateRequest = input.as_json()
            .context("Failed to parse migrate request")?;

        let response = perform_migration(request)?;

        Envelope::json(&response)
            .context("Failed to create response envelope")
    }
}

// Internal helper is fine (but goes through envelope publicly)
fn perform_migration(request: MigrateRequest) -> Result<MigrateResponse> {
    // ... actual logic ...
}
```

### CLI Usage (Steel Thread)

```rust
use morphir_builtins::migrate::MigrateExtension;
use morphir_ext_core::Envelope;

let migrate = MigrateExtension::default();

// Build request
let request = MigrateRequest {
    ir: load_ir()?,
    target_version: "v4".to_string(),
    expanded: false,
};

// Execute via envelope (only way)
let input = Envelope::json(&request)?;
let output = migrate.execute_native(&input)?;
let response: MigrateResponse = output.as_json()?;
```

## Alternatives Considered

### Alternative 1: Dual API (Native Fast Path)

Add both envelope and typed methods:

```rust
pub trait BuiltinExtension {
    fn execute_native(&self, input: &Envelope) -> Result<Envelope>;
    fn execute_typed(&self) -> Option<Box<dyn Any>>;  // Optional fast path
}
```

**Rejected because:**

- Adds complexity for marginal performance gain
- Two APIs to maintain and document
- Breaks architectural uniformity
- Premature optimization

### Alternative 2: Trait-Based Dispatch with Downcast

Runtime dispatch tries native first, falls back to envelope:

```rust
pub trait ExtensionRuntime {
    fn call_envelope(&mut self, ...) -> Result<Envelope>;
    fn call_native(&mut self, ...) -> Option<Result<Box<dyn Any>>>;
}
```

**Rejected because:**

- Complex type erasure and downcasting
- Harder to debug and maintain
- Fragile (easy to break with wrong types)
- Not truly zero-cost (downcast checks)

### Alternative 3: Macro-Generated Fast Path

Use macros to generate both envelope and direct implementations:

**Rejected because:**

- Macro complexity for unclear benefit
- Harder to understand and debug
- Still maintains dual API surface
- Overkill for problem that may not exist

## Monitoring and Review

### Metrics to Track

If/when we add telemetry:

- Time spent in serialization vs. processing
- Percentile latency for operations using builtins
- Memory allocation patterns

### Review Triggers

Revisit this ADR if:

1. Profiling shows >10% time in JSON ser/deser
2. User complaints about CLI performance
3. IDE integration shows latency issues
4. Tight evaluation loops added (not just one-time operations)

## References

- Design discussion: Jan 29, 2026 (steel thread planning)
- [Extension Architecture](../design/extensions/actor-based/12-wasm-extension-architecture-session.md)
- [Envelope Protocol](../design/extensions/actor-based/12-wasm-extension-architecture-session.md#envelope-protocol)
- Steel Thread issue: `morphir-rust-steel-thread`
- Builtins implementation: `morphir-rust-builtins-v2`

## Amendments

None yet.

---

**Note:** This ADR documents a design decision, not a law. If circumstances change (profiling data, new requirements,
etc.), we can amend or supersede this decision with a new ADR.
