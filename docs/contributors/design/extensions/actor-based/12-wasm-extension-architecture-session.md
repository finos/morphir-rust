---
layout: default
title: Wasm Extension Architecture (Interactive Session)
nav_order: 12
parent: Morphir Extensions
---

# Wasm Extension Architecture (Interactive Session)

**Status:** Draft  
**Version:** 0.2.0  
**Last Updated:** 2026-01-25

## Session framing

**Goal:** Update the extension and daemon design to include the hidden Extism runtime, the envelope protocol, TEA runtime semantics, and Kameo actors.  
**Outcome:** This document captures the updated architecture and ABI contracts as an interactive design session.

## Interactive design session

### Prompt

We want a unified extension model that supports design-time and runtime extensions, with a stable engine-agnostic ABI based on envelopes and TEA. Extism should be hidden behind the Morphir runtime. The daemon should host extension instances via Kameo actors.

### Key decisions

- **Hidden engine:** Extism is used internally and never appears in extension ABIs or SDKs.
- **Single protocol:** All extension calls use an Envelope with `header`, `content_type`, and `content` bytes.
- **TEA semantics:** Runtime extensions follow TEA-style init/update/subscriptions with JSON payload conventions.
- **Actor isolation:** Each operation runs inside a Kameo actor that owns runtime state and loaded programs.
- **Host-agnostic design:** The same ABI works across CLI, daemon, services, and browser hosts.

### Open questions

- Should TEA `update` use two envelopes (msg + model) or a merged JSON wrapper by default?
- How should subscription envelopes encode timers, filesystem watches, or external event streams?
- What is the minimal host-function surface for initial runtime extensions?

## Goals and non-goals

### Goals

- **Unified extension model** for Morphir:
  - Design-time extensions (frontends, backends, transforms, decorators)
  - Runtime extensions (IR evaluators, effect handlers, domain logic)
- **Portable execution** via WebAssembly:
  - Core Wasm (`wasm32-unknown-unknown`)
  - WebAssembly Component Model (where available)
- **Stable, host-agnostic protocol**:
  - Envelope: `header + content_type + content`
  - Works across CLI, daemon, services, browser, Wasmtime, GraalVM, Node, etc.
- **Predictable runtime semantics**:
  - Elm-style (TEA) architecture for runtime extensions
- **Isolation and concurrency**:
  - Kameo actors as the execution boundary for extensions
- **Hidden engine**:
  - Use Extism internally to manage Wasm plugins
  - Extension authors are **not aware** of Extism

### Non-goals

- Forcing extension authors to depend on Extism APIs or SDKs
- Locking Morphir into a single Wasm engine forever
- Exposing engine-specific details (Extism, Wasmtime, jco, etc.) in extension ABIs

## High-level architecture

```text
+-------------------------+
|        CLI Client       |
+-----------+-------------+
            |
            v
+-------------------------+
|         Daemon          |
|  (manages actors, I/O)  |
+-----------+-------------+
            |
            v
+-------------------------+
|       Kameo Actor       |
|  (extension instance)   |
+-----------+-------------+
            |
            v
+-------------------------+
|     Morphir Runtime     |
|  (Rust, uses Extism)    |
+-----------+-------------+
            |
            v
+-------------------------+
|    Extension Program    |
|   (Wasm, core/component)|
+-------------------------+
```

Key points:

- The **CLI** talks to a **daemon**.
- The **daemon** manages **Kameo actors**, one per operation/extension instance.
- Each **actor** hosts:
  - the **Morphir runtime** (Rust, compiled to native or Wasm)
  - one or more **extension programs** (Wasm modules)
- The runtime uses **Extism internally** to load and call extension programs.
- Extension authors only see:
  - the **envelope protocol**
  - the **TEA-style ABI**
  - the **design-time ABI**
  - the **standard library** (host functions)

## Core concepts

### Host

The **host** is any environment that runs the Morphir runtime:

- CLI process
- Daemon process
- Long-running service
- Browser (via jco, for a Wasm-compiled runtime)
- Wasmtime, GraalVM, Node/Deno/Bun

Responsibilities:

- Start and manage Kameo actors
- Provide host functions (logging, HTTP, filesystem, timers, etc.)
- Manage workspace and session state
- Route messages between CLI and actors

### Runtime

The **Morphir runtime** is a Rust library/binary that:

- Implements the **envelope protocol**
- Implements the **TEA runtime** for runtime extensions
- Implements the **design-time extension** interface
- Uses **Extism internally** to load and call Wasm extension programs
- Exposes a clean API to the host/actors (no Extism types leak out)

Conceptual structure:

```text
+----------------------------------+
|          Morphir Runtime         |
+----------------------------------+
|  +----------------------------+  |
|  |   Extism Integration       |  |  (hidden)
|  +----------------------------+  |
|  |   Program Registry         |  |
|  +----------------------------+  |
|  |   TEA Loop Engine          |  |
|  +----------------------------+  |
|  |   Envelope Codec           |  |
|  +----------------------------+  |
|  |   Host Function Glue       |  |
+----------------------------------+
```

### Program (Extension)

A **program** is a Wasm module that implements one of:

- **Design-time extension interfaces**:
  - `frontend-compile(input: Envelope) -> Envelope`
  - `backend-generate(input: Envelope) -> Envelope`
  - `get-capabilities() -> Envelope`
- **Runtime extension interface** (TEA):
  - `init(flags: Envelope) -> Envelope`
  - `update(msg: Envelope, model: Envelope) -> Envelope`
  - `subscriptions(model: Envelope) -> Envelope`

Programs may be:

- Core Wasm modules (simple `(ptr,len)` ABI)
- Component Model components (WIT-based, where supported)

Programs are dynamically loaded by the runtime via Extism.

## Envelope protocol

All messages between host, runtime, and programs use an **envelope**:

```rust
struct Envelope {
    header: Header,
    content_type: String,
    content: Vec<u8>,
}

struct Header {
    seqnum: u64,
    session_id: String,
    kind: Option<String>,
}
```

Conceptually:

```text
+----------------------------------------+
|               Envelope                 |
+----------------------------------------+
| header: { seqnum, session_id, kind? } |
| content_type: "application/json"      |
| content: <raw bytes>                  |
+----------------------------------------+
```

Header definition:

```text
+-------------------------------+
|            Header             |
+-------------------------------+
| seqnum: u64 = 0               |
| session_id: String = ""       |
| kind?: String (optional)      |
+-------------------------------+
```

Examples:

- `application/json`
- `application/morphir-ir+json`
- `application/morphir-ir+cbor`
- `text/typescript`
- `application/x-morphir-backend`

Benefits:

- Encoding-agnostic
- Extensible and versionable
- Works in core Wasm and Component Model
- Browser-friendly
- Uniform across design-time and runtime
- Carries lightweight metadata in `header` (sequence number, session ID, optional kind)

## TEA-style runtime (Elm-like architecture)

The runtime follows a TEA-like model:

```text
+---------------------------+
|         Program           |
|  (pure functions)         |
+---------------------------+
           ^
           | envelopes
           v
+---------------------------+
|         Runtime           |
|  (state + effects)        |
+---------------------------+
           ^
           | host functions
           v
+---------------------------+
|           Host            |
+---------------------------+
```

Program interface:

- `init(flags: Envelope) -> Envelope`  
  Returns an envelope containing `(model, cmds)`.

- `update(msg: Envelope, model: Envelope) -> Envelope`  
  Returns an envelope containing `(model, cmds)`.

- `subscriptions(model: Envelope) -> Envelope`  
  Returns an envelope describing subscriptions.

The runtime:

- Maintains per-program state (`model`)
- Interprets `cmds` and `subs` envelopes
- Calls host functions (standard library)
- Exposes `start`, `send`, `poll` to the host

## Hidden Extism engine

### Design principle

- **Extism is an implementation detail.**
- Extension authors:
  - do **not** import Extism functions
  - do **not** depend on Extism SDKs
  - only implement the Morphir-defined ABI (envelope + TEA/design-time)
- The runtime:
  - uses Extism to load and call Wasm modules
  - can be swapped out later for Wasmtime, jco, etc. without breaking extensions

### Envelope codec (host side)

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub seqnum: u64,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            seqnum: 0,
            session_id: String::new(),
            kind: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub header: Header,
    pub content_type: String,
    #[serde(with = "serde_bytes")]
    pub content: Vec<u8>,
}

impl Envelope {
    pub fn json<T: Serialize>(value: &T) -> anyhow::Result<Self> {
        Ok(Self {
            header: Header::default(),
            content_type: "application/json".to_string(),
            content: serde_json::to_vec(value)?,
        })
    }

    pub fn as_json<T: for<'de> Deserialize<'de>>(&self) -> anyhow::Result<T> {
        if self.content_type != "application/json" {
            anyhow::bail!("expected application/json, got {}", self.content_type);
        }
        Ok(serde_json::from_slice(&self.content)?)
    }
}

pub fn encode_envelope(env: &Envelope) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(env)?)
}

pub fn decode_envelope(bytes: &[u8]) -> anyhow::Result<Envelope> {
    Ok(serde_json::from_slice(bytes)?)
}
```

Extism only ever sees `Vec<u8>`; the runtime sees `Envelope`.

### Extism runtime wrapper

```rust
use extism::{Context, Manifest, Plugin};
use crate::envelope::{Envelope, encode_envelope, decode_envelope};

pub struct ExtismRuntime {
    ctx: Context,
    plugin: Plugin,
}

impl ExtismRuntime {
    pub fn new(wasm_bytes: Vec<u8>) -> anyhow::Result<Self> {
        let manifest = Manifest::new([wasm_bytes]);
        let ctx = Context::new();
        let plugin = ctx.new_plugin(manifest, true)?;
        Ok(Self { ctx, plugin })
    }

    pub fn call_envelope(&mut self, func: &str, input: &Envelope) -> anyhow::Result<Envelope> {
        let input_bytes = encode_envelope(input)?;
        let output_bytes = self.plugin.call(func, &input_bytes)?;
        let env = decode_envelope(&output_bytes)?;
        Ok(env)
    }
}
```

Everything above this layer is Extism-free.

## Extension ABIs

### Design-time extension ABI

Design-time capabilities are split into focused interfaces that still use the same `Envelope` ABI:

- **Frontend**: `frontend-compile(input: Envelope) -> Envelope`
- **Backend**: `backend-generate(input: Envelope) -> Envelope`
- **Common-backend**: `get-capabilities() -> Envelope`

## WIT definitions (current)

These are the current Component Model definitions used by `morphir-ext-core`.

### envelope.wit

```wit
package morphir:ext@0.1.0;

interface envelope {
    record header {
        seqnum: u64,
        session-id: string,
        kind: option<string>,
    }

    record envelope {
        header: header,
        content-type: string,
        content: list<u8>,
    }
}
```

### runtime.wit

```wit
package morphir:ext@0.1.0;

interface runtime {
    use envelope.{envelope};

    enum log-level {
        trace,
        debug,
        info,
        warn,
        error
    }

    variant env-value {
        text(string),
        text-list(list<string>),
        boolean(bool),
        v-u8(u8),
        v-u16(u16),
        v-u32(u32),
        v-u64(u64),
        v-s8(s8),
        v-s16(s16),
        v-s32(s32),
        v-s64(s64),
        v-f32(f32),
        v-f64(f64)
    }

    /// Log a message to the host.
    log: func(level: log-level, msg: string);

    /// Get an environment variable.
    get-env-var: func(name: string) -> option<env-value>;

    /// Set an environment variable.
    set-env-var: func(name: string, value: env-value);
}

world extension {
    import runtime;
    export program;
}
```

### program.wit

```wit
package morphir:ext@0.1.0;

interface program {
    use envelope.{envelope};

    /// Initialize the extension with startup flags.
    /// Returns the initial model and any initial commands.
    init: func(init-data: envelope) -> tuple<envelope, envelope>;

    /// Update the extension state based on a message.
    /// Returns the new model and any commands.
    update: func(msg: envelope, model: envelope) -> tuple<envelope, envelope>;

    /// Get active subscriptions based on the model.
    subscriptions: func(model: envelope) -> envelope;

    /// Get capabilities as a JSON envelope.
    get-capabilities: func() -> envelope;
}

interface design-time-frontend {
    use envelope.{envelope};

    /// Frontend compilation: source -> IR
    frontend-compile: func(input: envelope) -> envelope;
}

interface design-time-backend {
    use envelope.{envelope};

    /// Backend generation: IR -> artifacts
    backend-generate: func(input: envelope) -> envelope;
}

interface common-backend {
    use envelope.{envelope};

    /// Backend capability discovery.
    get-capabilities: func() -> envelope;
}
```

### Runtime extension ABI (TEA)

Extension authors implement:

```text
init(flags: Envelope) -> Envelope
update(msg: Envelope, model: Envelope) -> Envelope
subscriptions(model: Envelope) -> Envelope
```

We standardize the **payloads inside envelopes** as JSON objects:

- `init`:
  - Input: `flags` envelope (e.g. config, initial IR)
  - Output envelope: `application/json` with:
    ```json
    {
      "model": { ... },
      "cmds": [ ... ]
    }
    ```

- `update`:
  - Input envelopes: `msg`, `model`
  - Output envelope: `application/json` with:
    ```json
    {
      "model": { ... },
      "cmds": [ ... ]
    }
    ```

- `subscriptions`:
  - Input: `model` envelope
  - Output envelope: `application/json` with:
    ```json
    {
      "subs": [ ... ]
    }
    ```

The runtime wraps these in helper functions:

```rust
pub struct MorphirProgram {
    extism: ExtismRuntime,
}

impl MorphirProgram {
    pub fn init(&mut self, flags: Envelope) -> anyhow::Result<(Envelope, Envelope)> {
        let out = self.extism.call_envelope("init", &flags)?;
        // parse out.model, out.cmds from out.content
        // return (model_env, cmds_env)
        Ok((model_env, cmds_env))
    }

    pub fn update(&mut self, msg: Envelope, model: Envelope) -> anyhow::Result<(Envelope, Envelope)> {
        // combine msg + model into one envelope or define a convention
        // e.g. content_type = "application/json", content = { "msg": ..., "model": ... }
        let input = Envelope::json(&serde_json::json!({
            "msg": {
                "content_type": msg.content_type,
                "content": base64::encode(msg.content),
            },
            "model": {
                "content_type": model.content_type,
                "content": base64::encode(model.content),
            }
        }))?;
        let out = self.extism.call_envelope("update", &input)?;
        // parse out.model, out.cmds
        Ok((model_env, cmds_env))
    }

    pub fn subscriptions(&mut self, model: Envelope) -> anyhow::Result<Envelope> {
        self.extism.call_envelope("subscriptions", &model)
    }
}
```

Extension authors only see:

- `init/update/subscriptions` with `Envelope` arguments
- JSON payloads inside `Envelope.content` (if they choose)

## Standard library (host functions)

We provide a **standard library** to extensions via host functions, implemented in Rust and registered with Extism.

Examples:

- `log(env: Envelope)`
- `get_env_var(name: Envelope) -> Envelope`
- `set_env_var(name: Envelope, value: Envelope)`
- `random(env: Envelope) -> Envelope`
- `http(env: Envelope) -> Envelope`
- `workspace(env: Envelope) -> Envelope` (design-time only)
- `ir_helpers(env: Envelope) -> Envelope`

Conceptually:

```text
+---------------------------+
|      Standard Library     |
+---------------------------+
| log        (Envelope)     |
| get_env_var(Envelope)     |
| set_env_var(Envelope)     |
| random     (Envelope)     |
| http       (Envelope)     |
| workspace  (Envelope)     |
| ir_helpers (Envelope)     |
+---------------------------+
```

These are registered as Extism host functions, but extension authors only see them as imports in their language of choice (Rust, TypeScript, etc.) via a Morphir-specific SDK.

## Kameo actor model

Each **operation** (e.g. "run this transform", "compile this IR", "execute this program") is handled by a **Kameo actor**.

Actor responsibilities:

- Load the Morphir runtime
- Load one or more extension programs (via Extism)
- Maintain workspace and session state
- Execute TEA loops for runtime extensions
- Execute `process` calls for design-time extensions
- Interpret commands and subscriptions
- Call host functions (standard library)
- Stream progress and results back to the daemon

Actor structure:

```text
+----------------------------------+
|            Actor                 |
+----------------------------------+
|  +----------------------------+  |
|  |   Morphir Runtime          |  |
|  |   (uses Extism)            |  |
|  +----------------------------+  |
|  |   Loaded Extensions        |  |
|  +----------------------------+  |
|  |   Workspace / Session      |  |
|  +----------------------------+  |
|  |   Async Host Functions     |  |
+----------------------------------+
```

## CLI -> daemon -> actor flow

```text
+---------+        +-----------+        +-----------+
|   CLI   | -----> |  Daemon   | -----> |   Actor   |
+---------+        +-----------+        +-----------+
                        |                    |
                        | loads runtime       |
                        | loads extension(s)  |
                        | executes TEA or     |
                        | process()           |
                        | returns results     |
```

Typical flow:

1. CLI sends a request: "run transform X on workspace Y".
2. Daemon:
   - creates or reuses an actor for that workspace/session
   - instructs it to load extension X (if not already loaded)
3. Actor:
   - loads the extension via Extism
   - calls `process` or `init/update/subscriptions`
   - uses host functions as needed
   - returns results to daemon
4. Daemon streams results back to CLI.

## Browser and other hosts

Because Extism is hidden:

- In **native/daemon** environments:
  - Runtime uses Extism internally.
- In **browser** environments:
  - You can compile the runtime itself to Wasm and use jco or another host.
  - The extension ABI (envelope + TEA/design-time) stays the same.
  - You can replace Extism with:
    - direct `WebAssembly.instantiate`
    - jco for Component Model
    - Wasmtime in other contexts

The key: **extension authors never see the engine**.

## Implementation roadmap

1. **Envelope layer**
   - Implement `Envelope` + JSON codec (host side).
   - Define JSON payload conventions for TEA and design-time.

2. **Extism integration**
   - Implement `ExtismRuntime` wrapper (`call_envelope`).
   - Register basic host functions (log, random).

3. **TEA runtime**
   - Implement `MorphirProgram` on top of `ExtismRuntime`.
   - Implement `start`, `send`, `poll` semantics.

4. **Design-time runtime**
   - Implement `process` calls for design-time extensions.
   - Add a simple extension registry.

5. **Kameo actors**
   - Wrap runtime in actors.
   - Add workspace/session state.
   - Add message protocol between daemon and actors.

6. **CLI + daemon**
   - Define CLI commands.
   - Implement daemon routing to actors.

7. **Standard library**
   - Flesh out host functions (HTTP, workspace, IR helpers).
   - Provide language-specific SDKs for extension authors.

8. **Component Model compatibility (optional/next)**
    - Define WIT for envelope + TEA + design-time.
    - Provide wrappers for Component Model hosts (Wasmtime, jco).

## Recommended packaging

For most extensions, **one crate with two feature-gated builds** (design-time vs runtime) provides the lowest
maintenance burden while keeping the same Envelope ABI. This keeps shared types and helpers in one place, avoids
unnecessary dependencies in each build, and produces a focused export surface.

Choose a **single module that exports both interfaces** only when you need one Wasm artifact capable of both
design-time and runtime behavior at runtime.

## Summary

This design gives Morphir:

- A **unified extension model** for design-time and runtime.
- A **stable, engine-agnostic ABI** based on envelopes and TEA.
- A **hidden Extism integration** that simplifies implementation without leaking into extension APIs.
- A **Kameo actor-based host** that provides isolation, concurrency, and lifecycle management.
- A **standard library** exposed as host functions, not engine details.
- A path to **Component Model** and **browser execution** without breaking existing extensions.

## Related documents

- [Actor-Based Extension System Overview](./00-overview.md)
- [Extension Host Interface](./02-extension-host-interface.md)
- [Extism WASM Host](./06-extism-wasm-host.md)
- [Daemon Architecture](../../daemon/README.md)
