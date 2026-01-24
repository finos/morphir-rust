---
layout: default
title: Stdio Extension Host
nav_order: 7
parent: Morphir Extensions
---

# Stdio Extension Host

**Status:** Draft  
**Version:** 0.1.0

## Overview

The Stdio Extension Host manages extensions that communicate via JSON Lines over stdin/stdout. This is the simplest protocol, ideal for scripts and command-line tools written in any language.

## Protocol

### Transport

- **Input**: JSON Lines written to stdin
- **Output**: JSON Lines read from stdout
- **Errors**: Human-readable text written to stderr (logged by host)

### Message Format

#### Request (Host → Extension)

```json
{"id": 1, "method": "transform", "params": {"ir": {...}}}
```

Fields:

- `id` (number): Unique request ID for matching responses
- `method` (string): Method name to invoke
- `params` (any): Method parameters

#### Response (Extension → Host)

Success:

```json
{ "id": 1, "result": { "transformed": true } }
```

Error:

```json
{ "id": 1, "error": "Invalid IR structure" }
```

Fields:

- `id` (number): Matches request ID
- `result` (any): Success result (mutually exclusive with error)
- `error` (string): Error message (mutually exclusive with result)

#### Notification (Extension → Host, one-way)

```json
{ "method": "log", "params": { "level": "info", "message": "Processing..." } }
```

No `id` field, no response expected.

## Implementation

### Architecture

```
┌──────────────────┐
│ StdioExtensionHost│
│  (Kameo Actor)    │
└──────────┬─────────┘
           │
           ├──► Extension 1 (Process)
           │    ├─ stdin writer task
           │    ├─ stdout reader task
           │    └─ stderr reader task
           │
           ├──► Extension 2 (Process)
           │    └─ ...
           │
           └──► Extension N (Process)
```

### State

```rust
pub struct StdioExtensionHost {
    extensions: HashMap<ExtensionId, StdioExtension>,
    next_id: u64,
}

struct StdioExtension {
    name: String,
    process: Child,
    request_tx: mpsc::Sender<(StdioRequest, Option<oneshot::Sender<StdioResponse>>)>,
    pending: HashMap<u64, oneshot::Sender<StdioResponse>>,
    next_request_id: u64,
    capabilities: Vec<Capability>,
}
```

### Concurrency Model

Each extension has three async tasks:

1. **Stdin Writer**: Receives requests from host, writes JSON lines to stdin
2. **Stdout Reader**: Reads JSON lines from stdout, routes responses to pending requests
3. **Stderr Reader**: Reads text from stderr, logs with extension name prefix

Communication flow:

```
Host.call()
  ↓
Create oneshot channel
  ↓
Send (request, response_tx) to stdin writer
  ↓
Store response_tx in pending map
  ↓
Stdin writer serializes and writes
  ↓
... extension processes ...
  ↓
Stdout reader parses response
  ↓
Lookup response_tx by request id
  ↓
Send response through oneshot
  ↓
Host.call() returns
```

### Loading Process

```rust
async fn load_extension(&mut self, config: ExtensionConfig) -> Result<ExtensionId, ExtensionError> {
    // 1. Extract command and args
    let ExtensionSource::Process { command, args, env } = config.source else {
        return Err(ExtensionError::InvalidSource);
    };

    // 2. Spawn process
    let mut child = Command::new(&command)
        .args(&args)
        .envs(&env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true) // Kill on host drop
        .spawn()?;

    // 3. Take stdio handles
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // 4. Create channels
    let (request_tx, request_rx) = mpsc::channel(32);
    let (response_tx, mut response_rx) = mpsc::channel(32);

    // 5. Spawn tasks
    tokio::spawn(stdin_writer_task(stdin, request_rx));
    tokio::spawn(stdout_reader_task(stdout, response_tx));
    tokio::spawn(stderr_reader_task(stderr, config.name.clone()));

    // 6. Send initialize
    let init_response = self.send_request(
        &request_tx,
        0,
        "initialize",
        config.config.clone(),
    ).await?;

    // 7. Query capabilities
    let caps_response = self.send_request(
        &request_tx,
        1,
        "capabilities",
        Value::Null,
    ).await?;

    let capabilities = serde_json::from_value(caps_response.result.unwrap_or_default())?;

    // 8. Store and return
    let id = ExtensionId(self.next_id);
    self.next_id += 1;

    self.extensions.insert(id, StdioExtension {
        name: config.name,
        process: child,
        request_tx,
        pending: HashMap::new(),
        next_request_id: 2,
        capabilities,
    });

    Ok(id)
}
```

### Call Process

```rust
async fn call(
    &mut self,
    extension_id: ExtensionId,
    method: &str,
    params: Value,
) -> Result<Value, ExtensionError> {
    let ext = self.extensions
        .get_mut(&extension_id)
        .ok_or(ExtensionError::NotFound(extension_id))?;

    let request_id = ext.next_request_id;
    ext.next_request_id += 1;

    let request = StdioRequest {
        id: request_id,
        method: method.to_string(),
        params,
    };

    let (response_tx, response_rx) = oneshot::channel();
    ext.pending.insert(request_id, response_tx);

    // Send request
    ext.request_tx
        .send((request, None))
        .await
        .map_err(|_| ExtensionError::ProtocolError("Channel closed".into()))?;

    // Wait for response with timeout
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        response_rx
    ).await
    .map_err(|_| ExtensionError::Timeout)?
    .map_err(|_| ExtensionError::ProtocolError("Response channel closed".into()))?;

    ext.pending.remove(&request_id);

    // Check for error
    if let Some(error) = response.error {
        return Err(ExtensionError::ExtensionError(error));
    }

    Ok(response.result.unwrap_or(Value::Null))
}
```

## Extension Implementation Guide

### Minimal Python Extension

```python
#!/usr/bin/env python3
import sys
import json

def handle_initialize(params):
    return {"status": "ready"}

def handle_capabilities(params):
    return [
        {"name": "transform", "description": "Transform IR"},
    ]

def handle_transform(params):
    # Process IR
    return {"result": params}

HANDLERS = {
    "initialize": handle_initialize,
    "capabilities": handle_capabilities,
    "transform": handle_transform,
}

def main():
    for line in sys.stdin:
        try:
            request = json.loads(line)
            method = request["method"]
            params = request.get("params", {})

            if method in HANDLERS:
                result = HANDLERS[method](params)
                response = {"id": request["id"], "result": result}
            else:
                response = {"id": request["id"], "error": f"Unknown method: {method}"}

            print(json.dumps(response), flush=True)
        except Exception as e:
            error_response = {"id": request.get("id", 0), "error": str(e)}
            print(json.dumps(error_response), flush=True)
            print(f"Error: {e}", file=sys.stderr, flush=True)

if __name__ == "__main__":
    main()
```

### Minimal Go Extension

```go
package main

import (
    "bufio"
    "encoding/json"
    "fmt"
    "os"
)

type Request struct {
    ID     uint64          `json:"id"`
    Method string          `json:"method"`
    Params json.RawMessage `json:"params"`
}

type Response struct {
    ID     uint64      `json:"id"`
    Result interface{} `json:"result,omitempty"`
    Error  *string     `json:"error,omitempty"`
}

func handleInitialize(params json.RawMessage) (interface{}, error) {
    return map[string]string{"status": "ready"}, nil
}

func handleCapabilities(params json.RawMessage) (interface{}, error) {
    return []map[string]string{
        {"name": "transform", "description": "Transform IR"},
    }, nil
}

func handleTransform(params json.RawMessage) (interface{}, error) {
    var input map[string]interface{}
    json.Unmarshal(params, &input)
    return map[string]interface{}{"result": input}, nil
}

func main() {
    scanner := bufio.NewScanner(os.Stdin)
    encoder := json.NewEncoder(os.Stdout)

    handlers := map[string]func(json.RawMessage) (interface{}, error){
        "initialize":   handleInitialize,
        "capabilities": handleCapabilities,
        "transform":    handleTransform,
    }

    for scanner.Scan() {
        var req Request
        if err := json.Unmarshal(scanner.Bytes(), &req); err != nil {
            fmt.Fprintf(os.Stderr, "Parse error: %v\n", err)
            continue
        }

        handler, ok := handlers[req.Method]
        if !ok {
            errStr := fmt.Sprintf("Unknown method: %s", req.Method)
            encoder.Encode(Response{ID: req.ID, Error: &errStr})
            continue
        }

        result, err := handler(req.Params)
        if err != nil {
            errStr := err.Error()
            encoder.Encode(Response{ID: req.ID, Error: &errStr})
        } else {
            encoder.Encode(Response{ID: req.ID, Result: result})
        }
    }
}
```

### Minimal Gleam Extension

```gleam
import gleam/io
import gleam/json
import gleam/dynamic
import gleam/result
import gleam/string

pub type Request {
  Request(id: Int, method: String, params: json.Json)
}

pub type Response {
  Response(id: Int, result: Result(json.Json, String))
}

fn handle_initialize(_params: json.Json) -> Result(json.Json, String) {
  json.object([
    #("status", json.string("ready")),
  ])
  |> Ok
}

fn handle_capabilities(_params: json.Json) -> Result(json.Json, String) {
  json.array([
    json.object([
      #("name", json.string("transform")),
      #("description", json.string("Transform Morphir IR")),
    ]),
  ])
  |> Ok
}

fn handle_transform(params: json.Json) -> Result(json.Json, String) {
  json.object([
    #("transformed", json.bool(True)),
    #("output", params),
  ])
  |> Ok
}

pub fn main() {
  read_loop()
}

fn read_loop() {
  case io.get_line("") {
    Ok(line) -> {
      case handle_line(line) {
        Ok(response) -> {
          io.println(json.to_string(response))
          read_loop()
        }
        Error(err) -> {
          io.println_error(err)
          read_loop()
        }
      }
    }
    Error(_) -> Nil
  }
}

fn handle_line(line: String) -> Result(json.Json, String) {
  use request <- result.try(parse_request(line))

  let result = case request.method {
    "initialize" -> handle_initialize(request.params)
    "capabilities" -> handle_capabilities(request.params)
    "transform" -> handle_transform(request.params)
    _ -> Error("Unknown method: " <> request.method)
  }

  encode_response(request.id, result)
}
```

## Configuration Example

```toml
[[extensions]]
name = "python-validator"
enabled = true
protocol = "stdio"

[extensions.source]
type = "process"
command = "python3"
args = ["./extensions/validator.py"]

[extensions.permissions]
network = false
filesystem = []

[extensions.restart]
strategy = "exponential"
initial_delay = "1s"
max_delay = "30s"
max_retries = 3
```

## Testing

### Manual Testing

```bash
# Test extension directly
echo '{"id":1,"method":"initialize","params":{}}' | python3 extension.py
echo '{"id":2,"method":"capabilities","params":{}}' | python3 extension.py
echo '{"id":3,"method":"transform","params":{"ir":{}}}' | python3 extension.py
```

### Integration Test

```rust
#[tokio::test]
async fn test_stdio_extension() {
    let mut host = StdioExtensionHost::new();
    host.initialize().await.unwrap();

    let config = ExtensionConfig {
        name: "test".into(),
        source: ExtensionSource::Process {
            command: "python3".into(),
            args: vec!["./tests/fixtures/echo.py".into()],
            env: HashMap::new(),
        },
        permissions: Permissions::default(),
        config: json!({}),
        restart: RestartStrategy::Never,
    };

    let id = host.load_extension(config).await.unwrap();

    let result = host.call(id, "echo", json!({"msg": "hello"})).await.unwrap();
    assert_eq!(result, json!({"msg": "hello"}));

    host.unload_extension(id).await.unwrap();
}
```

## Performance Characteristics

- **Latency**: 5-20ms per call (depends on language startup)
- **Throughput**: 50-100 calls/sec per extension
- **Memory**: 10-100MB per extension (process overhead)
- **Startup**: 100-1000ms (depends on language)

## Best For

✅ Scripts and command-line tools  
✅ Rapid prototyping  
✅ Legacy tool integration  
✅ Any language with stdin/stdout  
✅ Debugging (easy to test manually)

❌ High-throughput scenarios  
❌ Low-latency requirements  
❌ Large data transfers (JSON serialization overhead)

## Related

### Morphir Rust Design Documents

- **[Morphir Extensions](../README.md)** - Extension system overview
- **[WASM Components](../wasm-component.md)** - Component model integration
- **[Tasks](../tasks.md)** - Task system definition

### Main Morphir Documentation

- [Morphir Documentation](https://morphir.finos.org) - Main Morphir documentation site
- [Morphir LLMs.txt](https://morphir.finos.org/llms.txt) - Machine-readable documentation index
- [Morphir IR v4 Design](https://morphir.finos.org/docs/design/draft/ir/) - IR v4 design documents
- [Morphir IR Specification](https://morphir.finos.org/docs/morphir-ir-specification/) - Complete IR specification
