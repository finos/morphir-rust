# Morphir Extension Development Guide

This guide describes how to develop Morphir extensions using various programming languages. Morphir extensions are WebAssembly (Wasm) components that follow the The Elm Architecture (TEA) pattern.

## Prerequisites

Regardless of the language you choose, you will need the following tools:
- [Wasmtime](https://wasmtime.dev/) (Runtime)
- [wit-bindgen](https://github.com/bytecodealliance/wit-bindgen) (Binding generator)
- [wit-component](https://github.com/bytecodealliance/wit-component) (Component encoder)

The WIT definitions for Morphir extensions are located in `crates/morphir-ext-core/wit`.

## WASI Compatibility and the Component Model

Morphir uses a hybrid architecture to balance modern features with toolchain compatibility:

- **Host (Preview 2)**: The Morphir Daemon provides a **WASI Preview 2 (P2)** environment. This is based on the WebAssembly Component Model, which allows us to use WIT for high-level interfaces.
- **Guests (Preview 1 / Unknown)**: 
  - Most language toolchains (Rust `wasm32-wasip1`, TinyGo) still generate **WASI Preview 1 (P1)** code. These can be run in our host using a "compatibility adapter" (shim).
  - For extensions that don't need system access (like pure logic or string transformations), we recommend the `wasm32-unknown-unknown` target. This produces a "pure" Wasm module with no system dependencies, making it faster to load and easier to distribute.

---

## 1. Rust

Rust provides the most mature support for Wasm Components.

### Setup
Add `wit-bindgen` to your `Cargo.toml`:
```toml
[dependencies]
wit-bindgen = "0.35.0"
```

### Implementation
Use the `bindgen!` macro to generate host and guest bindings.

```rust
wit_bindgen::generate!({
    world: "extension",
    path: "wit", // Path to the .wit files
});

struct MyExtension;

impl Guest for MyExtension {
    fn init(init_data: Envelope) -> (Envelope, Envelope) {
        // ... implementation
    }
    
    fn update(msg: Envelope, model: Envelope) -> (Envelope, Envelope) {
        // ... implementation
    }
    // ... other TEA methods
}

export!(MyExtension);
```

### Build
Target `wasm32-wasip1` or `wasm32-unknown-unknown`, then encode into a component.

---

## 2. TypeScript / JavaScript

TypeScript extensions are built using `jco` and `componentize-js`.

### Setup
Install the necessary tools:
```bash
npm install -g @bytecodealliance/jco @bytecodealliance/componentize-js
```

### Implementation
Create a module that matches the exported `program` interface.

```typescript
export const program = {
    init(initData) {
        const initialModel = { /* ... */ };
        return [initialModel, []];
    },
    
    update(msg, model) {
        // ... implementation
        return [newModel, commands];
    },
    
    subscriptions(model) {
        return [];
    },
    
    info() {
        return { content: "My TypeScript Extension" };
    }
};
```

### Build
Componentize the JavaScript using `jco`:
```bash
jco componentize app.js --wit wit --world extension -o extension.wasm
```

---

## 3. Python

Python extensions are built using `componentize-py`.

### Setup
Install `componentize-py`:
```bash
pip install componentize-py
```

### Implementation
Implement the `Extension` world using Python class-based or module-based exports.

```python
import extension

class MyExtension(extension.Extension):
    def init(self, init_data: extension.Envelope) -> (extension.Envelope, extension.Envelope):
        # ...
        return (model, commands)
        
    def update(self, msg: extension.Envelope, model: extension.Envelope) -> (extension.Envelope, extension.Envelope):
        # ...
        return (new_model, commands)
```

### Build
```bash
componentize-py -d wit -w extension componentize my_app -o extension.wasm
```

---

## 4. Go

Go extensions use `TinyGo` for small Wasm binaries and `wit-bindgen-go`.

### Setup
Install [TinyGo](https://tinygo.org/) and [wit-bindgen-go](https://github.com/bytecodealliance/wit-bindgen-go).

### Implementation
Generate bindings and implement the methods.

```go
package main

import (
    "morphir/ext/program"
    "morphir/ext/envelope"
)

type MyExtension struct{}

func (e *MyExtension) Init(initData envelope.Envelope) (envelope.Envelope, envelope.Envelope) {
    // ...
}

func (e *MyExtension) Update(msg envelope.Envelope, model envelope.Envelope) (envelope.Envelope, envelope.Envelope) {
    // ...
}

func main() {
    program.SetProgram(&MyExtension{})
}
```

### Build
```bash
tinygo build -o extension.wasm -target=wasi main.go
# Then use wit-component to encode
```

---

## Appendix: Low-Level ABI Specification

If you are developing for a language that doesn't yet have high-level Component Model toolchains, you can implement the **Core Wasm ABI** directly.

### Memory Convention
All structured data (strings, bytes, JSON) is passed using a `(pointer, length)` pair where both values are `i32`.

### Guest Exports
The extension must export the following functions:

| Function | Signature | Purpose |
| :--- | :--- | :--- |
| `init` | `(hdr_p, hdr_l, ct_p, ct_l, m_p, m_l) -> i32` | Initializes the extension. Returns a program ID. |
| `update` | `(id, hdr_p, hdr_l, ct_p, ct_l, m_p, m_l) -> void` | Sends a message to a specific program instance. |

### Host Imports
The host provides the following functions in the `env` module:

| Function | Signature | Purpose |
| :--- | :--- | :--- |
| `log` | `(level, ptr, len) -> void` | Logs a message. Level: 0=Trace, 1=Debug, 2=Info, 3=Warn, 4=Error. |
| `get_env_var`| `(k_p, k_l, o_p, o_l) -> void` | Retrieves an environment variable as JSON. |
| `set_env_var`| `(k_p, k_l, v_p, v_l) -> void` | Stores an environment variable as JSON. |

### Data Structures
- **Header**: Passed as a JSON string: `{"seqnum": u64, "session_id": "uuid", "kind": "hint"}`.
- **Envelope Content**: Passed as raw bytes.
- **Environment Value**: Passed as a JSON serialized `EnvValue` enum.

For further details, explore the implementation in `crates/morphir-ext-core/src/abi.rs`.
