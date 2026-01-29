---
layout: default
title: Wasm Extension Architecture (Interactive Session)
nav_order: 12
parent: Morphir Extensions
---

# Wasm Extension Architecture (Interactive Session)

**Status:** Draft
**Version:** 0.3.0
**Last Updated:** 2026-01-29

## Session framing

**Goal:** Update the extension and daemon design to include the hidden Extism runtime, the envelope protocol, TEA
runtime semantics, Kameo actors, and support for multiple client types (CLI, Morphir-Live using Dioxus framework, WASM
clients with core Wasm and WASM Component Model + WASI).
**Outcome:** This document captures the updated architecture and ABI contracts as an interactive design session,
including deployment modes for native, daemon, Dioxus-based multi-platform (web/desktop), core Wasm (browser and
non-browser), and WASM Component + WASI environments.

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
+---------------------------+     +---------------------------+     +---------------------------+
|      CLI Client           |     |   Morphir-Live            |     |  WASM Client              |
|   (native binary)         |     |  (Dioxus: Web/Desktop)    |     |  (Core/Component+WASI)    |
+------------+--------------+     +-------------+-------------+     +-------------+-------------+
             |                                  |                                 |
             |                                  |                                 |
             v                                  v                                 v
+---------------------------------------------------------------------------------------------+
|                                        Daemon/Host Layer                                    |
|                           (manages actors, I/O, routing, state)                            |
+---------------------------------------------------------------------------------------------+
             |                                  |                                 |
             v                                  v                                 v
+---------------------------+     +---------------------------+     +---------------------------+
|      Kameo Actor          |     |      Kameo Actor          |     |   Direct Runtime Call     |
|  (extension instance)     |     |  (extension instance)     |     | (browser, no daemon)      |
+------------+--------------+     +-------------+-------------+     +-------------+-------------+
             |                                  |                                 |
             v                                  v                                 v
+---------------------------------------------------------------------------------------------+
|                                     Morphir Runtime                                         |
|                          (Rust, uses Extism internally for extensions)                     |
|                        (can be compiled to native or Wasm itself)                          |
+---------------------------------------------------------------------------------------------+
             |                                  |                                 |
             v                                  v                                 v
+---------------------------+     +---------------------------+     +---------------------------+
|   Extension Program       |     |   Extension Program       |     |   Extension Program       |
| (Wasm core/component)     |     | (Wasm core/component)     |     | (Wasm core/component)     |
+---------------------------+     +---------------------------+     +---------------------------+
```

Key points:

- **Multiple client types** access the runtime:
    - **CLI Client**: Native binary for command-line usage
    - **Morphir-Live**: Multi-platform app from finos/morphir using Dioxus framework (targets web and desktop)
    - **WASM Client**: Runtime compiled to Wasm, supporting:
        - Core Wasm (browser or non-browser hosts)
        - WASM Component Model with WASI (Wasmtime, browser with polyfills, etc.)
- The **daemon/host layer** manages **Kameo actors**, one per operation/extension instance.
- **Direct runtime calls** are possible in browser environments without daemon (for WASM clients).
- Each **actor** (or direct call) hosts:
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

- **CLI process**: Native binary for command-line usage
- **Daemon process**: Long-running service managing multiple sessions
- **Morphir-Live**: Multi-platform application using Dioxus framework (Rust → Web/Desktop, from finos/morphir)
- **WASM Client**: Runtime compiled to Wasm, runnable in:
    - **Core Wasm**: Browser (via WebAssembly API), Wasmtime, wasmer, Node, Deno, Bun
    - **WASM Component Model + WASI**: Wasmtime, wasmer with WASI support, browser with WASI polyfills
- **Long-running service**: Backend services, API servers
- **Other embedded runtimes**: GraalVM, custom Wasm hosts

Responsibilities:

- Start and manage Kameo actors (daemon/service mode)
- **OR** Make direct runtime calls (browser/WASM client mode)
- Provide host functions (logging, HTTP, filesystem, timers, etc.)
- Manage workspace and session state
- Route messages between clients and actors (in daemon mode)
- Handle persistence and caching (in daemon mode)

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

- **Core Wasm modules** (wasm32-unknown-unknown):
    - Simple `(ptr,len)` ABI for passing bytes
    - Works in any Wasm host (browser, Node, Wasmtime, etc.)
    - No access to system resources unless provided by host imports
- **WASM Component Model** (WIT-based):
    - Type-safe interfaces defined via WIT (WebAssembly Interface Types)
    - Can include WASI (WebAssembly System Interface) for system access:
        - **WASI Preview 2**: Filesystem, sockets, environment, clocks, random
        - Enables portable system-level extensions
    - Requires Component Model-capable host (Wasmtime, wasmer, or polyfills)

Programs are dynamically loaded by the runtime via Extism (for core Wasm) or Component Model loaders (for WASM
Components).

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

## Client to runtime flows

### Daemon-based flow (CLI, Morphir-Live in remote mode)

```text
+-----------+        +-----------+        +-----------+
|  Client   | -----> |  Daemon   | -----> |   Actor   |
+-----------+        +-----------+        +-----------+
                          |                    |
                          | loads runtime       |
                          | loads extension(s)  |
                          | executes TEA or     |
                          | process()           |
                          | returns results     |
```

Typical flow:

1. Client (CLI, Morphir-Live, etc.) sends a request: "run transform X on workspace Y".
2. Daemon:
   - creates or reuses an actor for that workspace/session
   - instructs it to load extension X (if not already loaded)
3. Actor:
   - loads the extension via Extism
   - calls `process` or `init/update/subscriptions`
   - uses host functions as needed
   - returns results to daemon
4. Daemon streams results back to client.

### Direct runtime flow (WASM client, Morphir-Live standalone)

```text
+------------------+        +------------------+        +-------------------+
|   WASM Client    | -----> | Runtime (Wasm)   | -----> |  Extension (Wasm) |
+------------------+        +------------------+        +-------------------+
  (JS/TS, Rust,             (Rust compiled               (core/component)
   Python, etc.)             to Wasm)
```

Typical flow:

1. Client code (JS/TS, Rust, Python, etc.) calls runtime API with envelope.
2. Runtime (compiled to Wasm):
    - loads extension if not already loaded
    - calls extension function with envelope
    - interprets commands/subscriptions
    - calls host functions (browser APIs, WASI, custom imports)
3. Runtime returns result envelope to client.
4. Client handles result (update UI, process data, etc.).

**Variants**:

- **Core Wasm**: Uses `WebAssembly.instantiate` or equivalent host API
- **WASM Component + WASI**: Uses WIT bindings, can access filesystem, network, environment via WASI

## Client deployment modes

Because Extism is hidden and the envelope protocol is host-agnostic, multiple deployment modes are supported:

### Native/Daemon Mode

- **CLI Client**: Native binary communicating with daemon
- **Daemon**: Long-running service managing Kameo actors
- **Runtime**: Uses Extism internally to load extensions
- **Use cases**: Development, CI/CD, backend services

### Morphir-Live Progressive Web App

- **Client**: Progressive Web App from finos/morphir (Rust → Wasm)
- **Architecture**: Rust runtime compiled to Wasm, running in browser
- **Actor support**: Can run Kameo actors in Wasm or make direct calls
- **Daemon option**: Can connect to remote daemon or run standalone
- **Use cases**: Interactive exploration, live documentation, educational demos

### WASM Client (Portable Wasm)

- **Client**: Runtime compiled to Wasm, runnable in various environments
- **Direct calls**: No daemon required, runtime and extensions all Wasm
- **Variants**:
    - **Core Wasm (wasm32-unknown-unknown)**:
        - Browser: Via `WebAssembly.instantiate`
        - Node/Deno/Bun: Via WebAssembly APIs
        - Wasmtime/wasmer: Embedded in other applications
    - **WASM Component Model + WASI**:
        - Wasmtime: Full WASI support, filesystem, network
        - Wasmer: WASI support
        - Browser: Via WASI polyfills or jco
        - Enables richer host functions (file I/O, sockets, etc.)
- **Extism alternative**: Can replace Extism with:
    - Direct `WebAssembly.instantiate` for core Wasm
    - jco/wasm-tools for Component Model
    - Host-native Wasm APIs
- **Use cases**:
    - Embedded widgets in web apps
    - Static sites with client-side IR processing
    - Serverless/edge functions
    - Portable CLI tools (single Wasm binary)
    - Sandboxed execution environments

### Service/Backend Mode

- **Long-running services**: API servers, data processing pipelines
- **Actor pools**: Managed Kameo actors for concurrency
- **Persistence**: Cached compilation results, persistent workspace state
- **Use cases**: Production deployments, scalable processing

### Portable Runtime Mode

- **Other hosts**: Wasmtime, GraalVM, Node/Deno/Bun
- **Same ABI**: Envelope protocol + TEA/design-time interfaces
- **Engine flexibility**: Can swap Extism for host-specific Wasm implementation
- **Use cases**: Embedding in existing platforms, custom integrations

The key: **extension authors never see the engine** and **the same extension works across all deployment modes**.

## Crate structure

The implementation is organized into the following crates:

### `morphir-ext-core`

**Status:** ✅ Complete

Core protocol definitions shared by all extension implementations:

- `Envelope` struct with `Header`, `content_type`, and `content`
- JSON codec for envelope serialization/deserialization
- Core WASM ABI definitions (pointer-based memory access)
- WIT interface definitions (envelope.wit, program.wit, runtime.wit)

### `morphir-ext`

**Status:** 🔄 In Progress

Main extension runtime and execution infrastructure:

- ✅ `ExtensionRuntime` trait (abstraction over WASM engines)
- ✅ `ExtensionInstance` (wraps `ExtensionRuntime`, manages state)
- ✅ `ExtensionActor` (Kameo actor wrapper for daemon mode)
- 🔴 `ExtismRuntime` (Extism implementation of `ExtensionRuntime`)
- 🔴 `DirectRuntime` (direct execution without actors)
- 🔴 `DaemonClient` (IPC client connecting to daemon actors)
- 🔴 Host functions (log, HTTP, workspace, IR helpers)

**Key insight:** `morphir-ext` contains both:

- The **interface** (`ExtensionRuntime` trait)
- Multiple **implementations** (Extism, direct, daemon client)
- **Execution modes** (direct vs actor-based)

**Client flexibility:** All clients can choose their execution mode:

- **DirectRuntime**: Embedded execution (CLI, Morphir-Live, browser/WASM)
    - No daemon required
    - Extensions run in-process
    - Suitable for single-user, local execution
- **DaemonClient**: Remote execution via daemon (CLI, Morphir-Live, IDE)
    - Shared state and caching across sessions
    - File watching and incremental compilation
    - Multi-user/multi-project support

The choice depends on deployment needs, not client type.

### `morphir-daemon`

**Status:** 🔄 In Progress

Daemon service for long-running extension hosting:

- JSON-RPC server for CLI/IDE integration
- File watching and incremental compilation
- Extension registry and loading
- Actor pool management (using `morphir-ext` actors)
- Workspace and session state management

### `morphir-runtime`

**Status:** 🔴 Not Started

Runtime execution semantics (TEA-style runtime extensions):

- IR evaluation engine
- Effect handlers and interpreters
- Domain logic execution
- Uses `morphir-ext` for extension hosting

### `morphir-design`

**Status:** 🔴 Not Started

Design-time operations (frontends, backends, transforms):

- Frontend compilation (source → IR)
- Backend generation (IR → target code)
- IR transformations and decorators
- Uses `morphir-ext` for extension hosting

### `morphir-builtins`

**Status:** ✅ Complete (structure), 🔄 In Progress (migrate implementation)

Builtin extensions bundled with Morphir:

- ✅ `BuiltinExtension` trait (native + optional WASM)
- ✅ `BuiltinRegistry` for discovery
- 🔄 `migrate` extension (IR v3 ↔ v4 transformation)
- 🔴 Future: TypeScript backend, Scala backend, etc.

**Dual execution modes:**

- **Native**: Direct Rust implementation (always available, best performance)
- **WASM**: Compiled to WASM for extension architecture validation

**Key insight:** Builtins are **both** usable as Rust libraries AND as WASM extensions, providing flexibility and
testing coverage for the extension architecture.

## Steel Thread: Migrate Command

**Goal:** Build a working end-to-end slice of functionality using the `migrate` command as a design-time extension (IR
transform/backend).

### Why Migrate Command?

The `migrate` command transforms Morphir IR from one version to another, making it an ideal steel thread because:

- **Design-time operation**: IR → IR transform (backend pattern)
- **Self-contained**: No runtime execution needed
- **Real-world use case**: Actual production requirement
- **Tests the full stack**: Extension loading, envelope protocol, DirectRuntime, WASM execution

### Steel Thread Scope

Implement **minimal viable path** through the architecture:

```text
CLI ---> DirectRuntime ---> ExtismRuntime ---> migrate.wasm
         (morphir-ext)      (morphir-ext)      (extension)
              |
              v
         ExtensionInstance
         (state management)
```

### Implementation Steps (Steel Thread)

1. **Minimal ExtismRuntime** (morphir-rust-ext1)
    - Load WASM module via Extism
    - Implement `call_envelope()` only
    - Skip TEA helpers for now
    - Basic error handling

2. **Minimal DirectRuntime** (morphir-rust-direct1)
    - Wrap ExtismRuntime
    - Expose simple API: `execute(func, input) -> output`
    - No state management initially

3. **Minimal Host Functions** (morphir-rust-std1)
    - `log()` only for debugging
    - Skip HTTP, workspace, etc. for now

4. **Migrate Extension** (morphir-builtins)
    - ✅ Already created in `morphir-builtins` crate
    - Implements `BuiltinExtension` trait
    - Native Rust implementation available immediately
    - Can be compiled to WASM for architecture testing
    - Takes IR envelope → returns migrated IR envelope

5. **CLI Integration**
    - `morphir migrate` command can use either:
        - **Native mode**: Call `MigrateExtension::execute_native()` directly (fast)
        - **WASM mode**: Load via DirectRuntime + ExtismRuntime (validates architecture)
    - Start with native mode, add WASM option later

### Success Criteria (Steel Thread)

- [ ] `morphir migrate input.json output.json` works end-to-end
- [ ] Extension loaded via ExtismRuntime
- [ ] Envelope protocol used throughout
- [ ] No daemon required (DirectRuntime only)
- [ ] Demonstrates extension architecture viability

### After Steel Thread

Once the steel thread works, expand:

- Add full TEA runtime support
- Add DaemonClient for remote execution
- Add more host functions
- Build more complex extensions

## Implementation roadmap

### Phase 0: Steel Thread (P0 - FIRST!)

Build working end-to-end with `migrate` command:

1. Minimal ExtismRuntime (call_envelope only)
2. Minimal DirectRuntime (simple wrapper)
3. Minimal host functions (log only)
4. Migrate extension (IR v3 → v4)
5. CLI integration

**Target:** Working `morphir migrate` using extension architecture

### Phase 1: Core Runtime (P1)

1. ✅ **Envelope layer** (morphir-rust-env1)
    - `Envelope` + JSON codec implemented in `morphir-ext-core`
    - JSON payload conventions defined

2. 🔄 **Extism integration** (morphir-rust-ext1)
    - Implement `ExtismRuntime` in `morphir-ext/src/extism_runtime.rs`
    - Implement `ExtensionRuntime` trait for `ExtismRuntime`
    - Register basic host functions (log, random)

3. 🔄 **TEA runtime** (morphir-rust-tea1)
    - Already have TEA helpers in `ExtensionRuntime` trait
    - Add command interpreter
    - Add subscription manager
    - Implement `start`, `send`, `poll` semantics

4. **Design-time runtime** (morphir-rust-dt1)
    - Implement `process` calls for design-time extensions
    - Add extension registry in `morphir-daemon`
    - Support frontend-compile and backend-generate

5. **Standard library** (morphir-rust-std1)
    - Implement host functions in `morphir-ext/src/host_functions.rs`
    - HTTP, workspace, IR helpers
    - Register with Extism
    - Provide language-specific SDKs for extension authors

### Phase 2: Native Client Support (P1)

6. ✅ **Kameo actors** (morphir-rust-kam1)
    - `ExtensionActor` implemented in `morphir-ext/src/actor.rs`
    - Wraps `ExtensionInstance`
    - Message handlers for init/update/subscriptions

7. **Direct runtime** (NEW)
    - Implement `DirectRuntime` in `morphir-ext/src/direct.rs`
    - For browser/WASM mode (no actors, no IPC)
    - Wraps `ExtensionInstance` directly

8. **Daemon client** (NEW)
    - Implement `DaemonClient` in `morphir-ext/src/daemon_client.rs`
    - IPC to actor-based daemon
    - Implements `ExtensionRuntime` trait

9. **CLI + daemon**
    - Define CLI commands in `morphir` crate
    - Implement daemon routing to actors in `morphir-daemon`
    - Add session management and persistence

### Phase 3: WASM Client Support (P2)

8. **Runtime → Core WASM compilation**
    - Compile Morphir runtime to core Wasm (wasm32-unknown-unknown).
    - Test core runtime APIs in multiple environments:
        - Browser (via WebAssembly API)
        - Node/Deno/Bun (via WebAssembly APIs)
        - Wasmtime/wasmer (embedded mode)
    - Ensure Extism or alternative works in Wasm context.

9. **WASM client interface**
    - Define language bindings for runtime API:
        - JavaScript/TypeScript (for browser, Node, Deno, Bun)
        - Rust (via wasm-bindgen for browser, direct for Wasmtime)
        - Python (via wasmtime-py or similar)
    - Implement envelope protocol over language boundaries.
    - Add environment-appropriate host functions:
        - Browser: IndexedDB, fetch, localStorage
        - WASI: Filesystem, sockets, environment variables
        - Custom: Host-specific imports

10. **Morphir-Live integration (Dioxus framework)**
    - Create integration layer for finos/morphir's Morphir-Live app (Dioxus-based).
    - Support desktop mode (native binary with embedded runtime).
    - Support web mode (Rust → Wasm with direct runtime calls).
    - Support both standalone mode (embedded runtime) and daemon mode (connect to remote).
    - Add web-specific features (offline, service worker, caching) for web target.

### Phase 4: Component Model & Advanced Features (P2-P3)

11. **WASM Component Model + WASI support**
    - Define WIT for envelope + TEA + design-time.
    - Compile runtime as WASM Component (with WASI preview 2).
    - Provide wrappers for Component Model hosts:
        - Wasmtime (native WASI support)
        - Wasmer (WASI support)
        - Browser (via WASI polyfills or jco)
    - Implement WASI-based host functions:
        - Filesystem operations (wasi:filesystem)
        - Network sockets (wasi:sockets)
        - Environment variables (wasi:cli-base)
    - Test Component Model extensions across all client types.

12. **Cross-client testing**
    - Create test suite that runs on:
        - CLI (native)
        - Daemon (native)
        - Morphir-Live (Dioxus: web/desktop)
        - WASM clients (core Wasm: browser, Node, Wasmtime)
        - WASM Component clients (WASI: Wasmtime, wasmer)
    - Verify envelope protocol works consistently across all targets.
    - Benchmark performance across deployment modes:
        - Native vs core Wasm vs WASM Component
        - Browser vs Node vs Wasmtime
        - Standalone vs daemon mode
    - Test Dioxus web target with both standalone and daemon modes.
    - Validate WASI host functions work correctly in Wasmtime/wasmer.

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
- A **Kameo actor-based host** that provides isolation, concurrency, and lifecycle management (for daemon/service mode).
- A **standard library** exposed as host functions, not engine details.
- **Multiple deployment modes**:
    - Native CLI + daemon for development and backend services
    - Morphir-Live (Dioxus) for web and desktop interactive exploration
    - Core WASM clients for embedded, browser, Node/Deno/Bun, and edge deployments
    - WASM Component + WASI for portable CLI tools and sandboxed execution
    - Service/backend mode for production deployments
- A path to **Component Model + WASI** and **portable Wasm execution** without breaking existing extensions.
- **Write once, run anywhere**: The same extension works across:
    - Native: CLI, daemon, services
    - Dioxus: Morphir-Live (web/desktop)
    - Core Wasm: Browser, Node, Deno, Bun, Wasmtime, wasmer
    - WASM Component + WASI: Wasmtime, wasmer, edge runtimes

## Related documents

- [Actor-Based Extension System Overview](./00-overview.md)
- [Extension Host Interface](./02-extension-host-interface.md)
- [Extism WASM Host](./06-extism-wasm-host.md)
- [Daemon Architecture](../../daemon/README.md)
