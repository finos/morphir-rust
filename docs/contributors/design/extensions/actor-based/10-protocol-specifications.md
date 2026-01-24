---
layout: default
title: Protocol Specifications
nav_order: 12
parent: Morphir Extensions
---

# Protocol Specifications

**Status:** Draft  
**Version:** 0.1.0

## Overview

This document specifies the wire protocols and message formats for all extension host types. These specifications ensure interoperability between Morphir and extensions.

## Common Lifecycle

All protocols follow a common lifecycle:

1. **Connection/Spawn**: Host connects to or spawns extension
2. **Initialize**: Host sends initialization message with configuration
3. **Capabilities**: Host queries extension capabilities
4. **Ready**: Extension is ready to process requests
5. **Call**: Zero or more method calls
6. **Shutdown**: Host notifies extension of shutdown
7. **Terminate**: Extension cleans up and exits

## Stdio Protocol (JSON Lines)

### Transport

- **Format**: JSON Lines (one JSON object per line)
- **Encoding**: UTF-8
- **Delimiter**: Newline character (`\n`)

### Request Format

```json
{
  "id": 123,
  "method": "transform",
  "params": {
    "ir": {...}
  }
}
```

**Fields:**

- `id` (integer): Unique request ID, must be echoed in response
- `method` (string): Method name to invoke
- `params` (any): Method-specific parameters

### Response Format

Success:

```json
{
  "id": 123,
  "result": {
    "output": "..."
  }
}
```

Error:

```json
{
  "id": 123,
  "error": "Error message"
}
```

**Fields:**

- `id` (integer): Matches request ID
- `result` (any): Result value (mutually exclusive with `error`)
- `error` (string): Error message (mutually exclusive with `result`)

### Notification Format

```json
{
  "method": "log",
  "params": {
    "level": "info",
    "message": "Processing..."
  }
}
```

**Note**: No `id` field, no response expected.

### Required Methods

All extensions must implement:

#### `initialize`

```json
{
  "id": 0,
  "method": "initialize",
  "params": {
    "config": {...}
  }
}
```

Response:

```json
{
  "id": 0,
  "result": {
    "status": "ready"
  }
}
```

#### `capabilities`

```json
{
  "id": 1,
  "method": "capabilities",
  "params": {}
}
```

Response:

```json
{
  "id": 1,
  "result": [
    {
      "name": "transform",
      "description": "Transform Morphir IR"
    }
  ]
}
```

## JSON-RPC 2.0 Protocol

### Transport

- **Format**: JSON-RPC 2.0
- **Transport**: HTTP POST
- **Content-Type**: `application/json`

### Request Format

```json
{
  "jsonrpc": "2.0",
  "id": 123,
  "method": "morphir.transform",
  "params": {
    "ir": {...}
  }
}
```

### Response Format

Success:

```json
{
  "jsonrpc": "2.0",
  "id": 123,
  "result": {
    "output": "..."
  }
}
```

Error:

```json
{
  "jsonrpc": "2.0",
  "id": 123,
  "error": {
    "code": -32000,
    "message": "Error message",
    "data": {...}
  }
}
```

### Required Methods

- `initialize`: Initialize extension
- `capabilities`: Return extension capabilities
- Custom methods as declared in capabilities

## gRPC Protocol

### Service Definition

```protobuf
syntax = "proto3";

package morphir.extension;

service ExtensionService {
  rpc Initialize(InitializeRequest) returns (InitializeResponse);
  rpc GetCapabilities(Empty) returns (CapabilitiesResponse);
  rpc Call(CallRequest) returns (CallResponse);
}

message InitializeRequest {
  string config_json = 1;
}

message InitializeResponse {
  string status = 1;
}

message Empty {}

message CapabilitiesResponse {
  repeated Capability capabilities = 1;
}

message Capability {
  string name = 1;
  string description = 2;
  optional string params_schema = 3;
  optional string return_schema = 4;
}

message CallRequest {
  string method = 1;
  bytes params_json = 2;
}

message CallResponse {
  oneof result {
    bytes success_json = 1;
    string error = 2;
  }
}
```

## Extism WASM Protocol

### Exported Functions

Extensions must export these functions:

```rust
// Initialize extension
#[plugin_fn]
pub fn initialize(config_json: String) -> FnResult<String> {
  // Returns: {"status": "ready"}
}

// Return capabilities
#[plugin_fn]
pub fn capabilities() -> FnResult<String> {
  // Returns: [{"name": "...", "description": "..."}]
}

// Custom methods
#[plugin_fn]
pub fn transform(params_json: String) -> FnResult<String> {
  // Returns: result JSON
}
```

### Data Format

All data is passed as JSON strings:

**Input**: JSON string containing parameters
**Output**: JSON string containing result

**Error Handling**: Return error via Extism error mechanism:

```rust
Err(extism_pdk::Error::msg("error message"))
```

## WASM Component Protocol

### Interface Definition (WIT)

```wit
package morphir:extension;

interface extension {
  record capability {
    name: string,
    description: string,
    params-schema: option<string>,
    return-schema: option<string>,
  }

  record init-result {
    status: string,
  }

  initialize: func(config: string) -> init-result;
  capabilities: func() -> list<capability>;

  // Custom methods
  transform: func(params: string) -> result<string, string>;
}

world morphir-extension {
  export extension;
}
```

### Error Handling

Methods return `result<T, string>` where:

- `ok(T)`: Success with value
- `err(string)`: Error with message

## Capability Schema

All protocols use a common capability format:

```json
{
  "name": "transform",
  "description": "Transform Morphir IR to target language",
  "params_schema": {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "properties": {
      "ir": {
        "type": "object",
        "description": "Morphir IR"
      },
      "options": {
        "type": "object",
        "description": "Generation options"
      }
    },
    "required": ["ir"]
  },
  "return_schema": {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "properties": {
      "output": {
        "type": "string",
        "description": "Generated code"
      }
    },
    "required": ["output"]
  }
}
```

**Fields:**

- `name` (string, required): Method name
- `description` (string, required): Human-readable description
- `params_schema` (object, optional): JSON Schema for parameters
- `return_schema` (object, optional): JSON Schema for return value

## Standard Methods

All extensions should implement these standard methods:

### `initialize`

Initialize the extension with configuration.

**Parameters:**

```json
{
  "config": {
    // Extension-specific configuration
  }
}
```

**Returns:**

```json
{
  "status": "ready"
}
```

### `capabilities`

Return list of capabilities.

**Parameters:** None

**Returns:**

```json
[
  {
    "name": "method_name",
    "description": "Method description",
    "params_schema": {...},
    "return_schema": {...}
  }
]
```

### `health_check` (Optional)

Check extension health.

**Parameters:** None

**Returns:**

```json
{
  "status": "healthy" | "degraded" | "unhealthy",
  "message": "Optional status message"
}
```

## Error Codes

Standard error codes for all protocols:

| Code             | Name             | Description                             |
| ---------------- | ---------------- | --------------------------------------- |
| -32600           | Invalid Request  | Invalid JSON or missing required fields |
| -32601           | Method Not Found | Method does not exist                   |
| -32602           | Invalid Params   | Invalid method parameters               |
| -32603           | Internal Error   | Internal extension error                |
| -32000 to -32099 | Server Error     | Extension-specific errors               |

## Versioning

Protocol version is declared in initialization:

```json
{
  "protocol_version": "1.0",
  "extension_version": "2.1.0"
}
```

**Compatibility Rules:**

- Major version change: Breaking changes
- Minor version change: Backward-compatible additions
- Patch version change: Bug fixes only

## Testing

All protocol implementations should pass these tests:

1. **Initialize Test**: Extension accepts initialize and returns success
2. **Capabilities Test**: Extension returns non-empty capabilities list
3. **Echo Test**: Extension echoes parameters back
4. **Error Test**: Extension returns proper error format
5. **Concurrent Test**: Extension handles multiple concurrent calls
6. **Timeout Test**: Extension respects timeout limits

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
