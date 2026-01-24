---
layout: default
title: Examples and Recipes
nav_order: 13
parent: Morphir Extensions
---

# Examples and Recipes

**Status:** Draft  
**Version:** 0.1.0

## Overview

This document provides practical examples and common patterns for building Morphir extensions.

## Quick Start Examples

### Minimal Python Extension (Stdio)

```python
#!/usr/bin/env python3
import sys
import json

def main():
    for line in sys.stdin:
        request = json.loads(line)
        method = request["method"]

        if method == "initialize":
            response = {"id": request["id"], "result": {"status": "ready"}}
        elif method == "capabilities":
            response = {"id": request["id"], "result": [
                {"name": "hello", "description": "Say hello"}
            ]}
        elif method == "hello":
            name = request["params"].get("name", "World")
            response = {"id": request["id"], "result": f"Hello, {name}!"}
        else:
            response = {"id": request["id"], "error": f"Unknown method: {method}"}

        print(json.dumps(response), flush=True)

if __name__ == "__main__":
    main()
```

**Configuration:**

```toml
[[extensions]]
name = "hello-python"
protocol = "stdio"
source = { type = "process", command = "python3", args = ["./hello.py"] }
```

### Minimal TypeScript Extension (JSON-RPC)

```typescript
import { createServer } from "jayson";

const server = createServer({
  initialize: (params: any, callback: any) => {
    callback(null, { status: "ready" });
  },

  capabilities: (params: any, callback: any) => {
    callback(null, [{ name: "greet", description: "Greet someone" }]);
  },

  greet: (params: { name: string }, callback: any) => {
    callback(null, { message: `Hello, ${params.name}!` });
  },
});

server.http().listen(3000);
console.log("Extension listening on port 3000");
```

**Configuration:**

```toml
[[extensions]]
name = "hello-typescript"
protocol = "jsonrpc"
source = { type = "http", url = "http://localhost:3000" }
```

### Minimal Rust Extension (Extism WASM)

```rust
use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct GreetParams {
    name: String,
}

#[derive(Serialize)]
struct GreetResult {
    message: String,
}

#[plugin_fn]
pub fn initialize(_config: String) -> FnResult<String> {
    Ok(r#"{"status": "ready"}"#.to_string())
}

#[plugin_fn]
pub fn capabilities() -> FnResult<String> {
    Ok(r#"[{"name": "greet", "description": "Greet someone"}]"#.to_string())
}

#[plugin_fn]
pub fn greet(params_json: String) -> FnResult<String> {
    let params: GreetParams = serde_json::from_str(&params_json)?;
    let result = GreetResult {
        message: format!("Hello, {}!", params.name),
    };
    Ok(serde_json::to_string(&result)?)
}
```

**Build:**

```bash
cargo build --target wasm32-unknown-unknown --release
```

**Configuration:**

```toml
[[extensions]]
name = "hello-rust"
protocol = "extism"
source = { type = "wasm", path = "./target/wasm32-unknown-unknown/release/hello.wasm" }
```

## Common Patterns

### Pattern: IR Transformer

Transform Morphir IR to another format.

```python
import sys
import json

def transform_ir(ir):
    # Transform logic here
    return {
        "transformed": True,
        "output": ir  # Replace with actual transformation
    }

def main():
    for line in sys.stdin:
        request = json.loads(line)
        method = request["method"]

        if method == "transform":
            ir = request["params"]["ir"]
            result = transform_ir(ir)
            response = {"id": request["id"], "result": result}
        # ... other methods

        print(json.dumps(response), flush=True)

if __name__ == "__main__":
    main()
```

### Pattern: IR Validator

Validate Morphir IR for custom business rules.

```python
def validate_ir(ir):
    errors = []
    warnings = []

    # Validation logic
    if not ir.get("modules"):
        errors.append("No modules found")

    # Check for naming conventions
    for module in ir.get("modules", []):
        if not module["name"].isupper():
            warnings.append(f"Module {module['name']} should be uppercase")

    return {
        "valid": len(errors) == 0,
        "errors": errors,
        "warnings": warnings
    }

def main():
    for line in sys.stdin:
        request = json.loads(line)

        if request["method"] == "validate":
            ir = request["params"]["ir"]
            result = validate_ir(ir)
            response = {"id": request["id"], "result": result}
            print(json.dumps(response), flush=True)
```

### Pattern: Code Generator

Generate code from Morphir IR.

```typescript
import { createServer } from "jayson";

function generateCode(ir: any, options: any): string {
  // Code generation logic
  const lines: string[] = [];

  for (const module of ir.modules) {
    lines.push(`export module ${module.name} {`);

    for (const func of module.functions) {
      lines.push(`  export function ${func.name}() {`);
      lines.push(`    // Generated from Morphir IR`);
      lines.push(`  }`);
    }

    lines.push(`}`);
  }

  return lines.join("\n");
}

const server = createServer({
  generate: (params: any, callback: any) => {
    try {
      const code = generateCode(params.ir, params.options || {});
      callback(null, { code });
    } catch (error) {
      callback(error);
    }
  },
});

server.http().listen(3000);
```

### Pattern: Stateful Extension

Maintain state across calls.

```python
import sys
import json

class StatefulExtension:
    def __init__(self):
        self.state = {}

    def set_state(self, params):
        key = params["key"]
        value = params["value"]
        self.state[key] = value
        return {"success": True}

    def get_state(self, params):
        key = params["key"]
        return {"value": self.state.get(key)}

    def clear_state(self, params):
        self.state = {}
        return {"success": True}

def main():
    extension = StatefulExtension()
    handlers = {
        "set_state": extension.set_state,
        "get_state": extension.get_state,
        "clear_state": extension.clear_state,
    }

    for line in sys.stdin:
        request = json.loads(line)
        method = request["method"]

        if method in handlers:
            result = handlers[method](request["params"])
            response = {"id": request["id"], "result": result}
        else:
            response = {"id": request["id"], "error": f"Unknown method: {method}"}

        print(json.dumps(response), flush=True)

if __name__ == "__main__":
    main()
```

### Pattern: Async Processing

Handle long-running operations asynchronously.

```typescript
import { createServer } from "jayson";
import { EventEmitter } from "events";

const jobs = new Map<string, EventEmitter>();

const server = createServer({
  // Start async job
  startJob: (params: any, callback: any) => {
    const jobId = crypto.randomUUID();
    const emitter = new EventEmitter();
    jobs.set(jobId, emitter);

    // Simulate long-running work
    setTimeout(() => {
      emitter.emit("complete", { result: "Job completed!" });
    }, 5000);

    callback(null, { jobId, status: "started" });
  },

  // Check job status
  getJobStatus: (params: { jobId: string }, callback: any) => {
    const emitter = jobs.get(params.jobId);
    if (!emitter) {
      callback(new Error("Job not found"));
      return;
    }

    callback(null, { status: "running" });
  },

  // Get job result (blocks until complete)
  getJobResult: (params: { jobId: string }, callback: any) => {
    const emitter = jobs.get(params.jobId);
    if (!emitter) {
      callback(new Error("Job not found"));
      return;
    }

    emitter.once("complete", (data) => {
      jobs.delete(params.jobId);
      callback(null, data);
    });
  },
});

server.http().listen(3000);
```

## Real-World Examples

### Example: TypeScript Code Generator

A complete TypeScript backend generator.

```typescript
// extension.ts
import { createServer } from "jayson";
import * as ts from "typescript";

interface MorphirType {
  type: "function" | "record" | "custom";
  name: string;
  fields?: any[];
}

class TypeScriptGenerator {
  generate(ir: any): string {
    const sourceFile = ts.createSourceFile(
      "output.ts",
      "",
      ts.ScriptTarget.Latest,
      false,
      ts.ScriptKind.TS,
    );

    const statements: ts.Statement[] = [];

    // Generate types
    for (const type of ir.types) {
      statements.push(this.generateType(type));
    }

    // Generate functions
    for (const func of ir.functions) {
      statements.push(this.generateFunction(func));
    }

    const printer = ts.createPrinter();
    return statements
      .map((s) => printer.printNode(ts.EmitHint.Unspecified, s, sourceFile))
      .join("\n\n");
  }

  private generateType(type: MorphirType): ts.Statement {
    // Type generation logic
    return ts.factory.createTypeAliasDeclaration(
      [ts.factory.createModifier(ts.SyntaxKind.ExportKeyword)],
      type.name,
      undefined,
      ts.factory.createTypeLiteralNode([]),
    );
  }

  private generateFunction(func: any): ts.Statement {
    // Function generation logic
    return ts.factory.createFunctionDeclaration(
      [ts.factory.createModifier(ts.SyntaxKind.ExportKeyword)],
      undefined,
      func.name,
      undefined,
      [],
      undefined,
      ts.factory.createBlock([]),
    );
  }
}

const generator = new TypeScriptGenerator();

const server = createServer({
  generate: (params: any, callback: any) => {
    try {
      const code = generator.generate(params.ir);
      callback(null, { code, language: "typescript" });
    } catch (error) {
      callback(error);
    }
  },
});

server.http().listen(3000);
```

### Example: Custom Linter

Lint Morphir IR for organization-specific rules.

```python
#!/usr/bin/env python3
import sys
import json
from typing import List, Dict, Any

class MorphirLinter:
    def __init__(self):
        self.rules = [
            self.check_naming_conventions,
            self.check_function_complexity,
            self.check_documentation,
        ]

    def lint(self, ir: Dict[str, Any]) -> Dict[str, Any]:
        errors = []
        warnings = []

        for rule in self.rules:
            rule_errors, rule_warnings = rule(ir)
            errors.extend(rule_errors)
            warnings.extend(rule_warnings)

        return {
            "valid": len(errors) == 0,
            "errors": errors,
            "warnings": warnings,
            "error_count": len(errors),
            "warning_count": len(warnings)
        }

    def check_naming_conventions(self, ir: Dict) -> tuple:
        errors = []
        warnings = []

        for module in ir.get("modules", []):
            # Module names should be PascalCase
            if not module["name"][0].isupper():
                warnings.append({
                    "rule": "naming-convention",
                    "message": f"Module '{module['name']}' should start with uppercase",
                    "location": module.get("location")
                })

        return errors, warnings

    def check_function_complexity(self, ir: Dict) -> tuple:
        errors = []
        warnings = []

        # Check cyclomatic complexity
        # (simplified for example)

        return errors, warnings

    def check_documentation(self, ir: Dict) -> tuple:
        errors = []
        warnings = []

        for module in ir.get("modules", []):
            if not module.get("documentation"):
                warnings.append({
                    "rule": "missing-documentation",
                    "message": f"Module '{module['name']}' lacks documentation",
                    "location": module.get("location")
                })

        return errors, warnings

def main():
    linter = MorphirLinter()

    for line in sys.stdin:
        request = json.loads(line)
        method = request["method"]

        if method == "lint":
            ir = request["params"]["ir"]
            result = linter.lint(ir)
            response = {"id": request["id"], "result": result}
        else:
            response = {"id": request["id"], "error": f"Unknown method: {method}"}

        print(json.dumps(response), flush=True)

if __name__ == "__main__":
    main()
```

## Testing Extensions

### Unit Testing (Python)

```python
import unittest
import json
from io import StringIO
from extension import handle_request

class TestExtension(unittest.TestCase):
    def test_initialize(self):
        request = {"id": 1, "method": "initialize", "params": {}}
        response = handle_request(request)
        self.assertEqual(response["result"]["status"], "ready")

    def test_capabilities(self):
        request = {"id": 2, "method": "capabilities", "params": {}}
        response = handle_request(request)
        self.assertIsInstance(response["result"], list)

    def test_transform(self):
        request = {
            "id": 3,
            "method": "transform",
            "params": {"ir": {"type": "Module"}}
        }
        response = handle_request(request)
        self.assertIn("output", response["result"])

if __name__ == '__main__':
    unittest.main()
```

### Integration Testing (Bash)

```bash
#!/bin/bash

# Test extension manually
echo '{"id":1,"method":"initialize","params":{}}' | python3 extension.py
echo '{"id":2,"method":"capabilities","params":{}}' | python3 extension.py
echo '{"id":3,"method":"echo","params":{"message":"test"}}' | python3 extension.py
```

## Configuration Recipes

### Development Configuration

```toml
[[extensions]]
name = "dev-generator"
enabled = true
protocol = "stdio"
source = { type = "process", command = "python3", args = ["./generator.py", "--verbose"] }
permissions = { filesystem = ["./output", "./temp"] }
restart = { strategy = "immediate", max_retries = 0 }
```

### Production Configuration

```toml
[[extensions]]
name = "prod-generator"
enabled = true
protocol = "grpc"
source = { type = "grpc", endpoint = "https://generator.internal:50051" }
permissions = { network = true, max_execution_time = "30s" }
restart = { strategy = "exponential", initial_delay = "1s", max_delay = "60s", max_retries = 5 }
```

### Sandboxed Configuration (WASM)

```toml
[[extensions]]
name = "untrusted-transformer"
enabled = true
protocol = "extism"
source = { type = "wasm", path = "./extensions/transformer.wasm" }
permissions = { max_memory = "50MB", max_execution_time = "5s" }
restart = { strategy = "never" }
```

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
