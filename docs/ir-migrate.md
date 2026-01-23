# IR Migrate Command

The `morphir ir migrate` command converts Morphir IR between format versions. This is useful when upgrading projects from Classic format (V1-V3) to V4 format, or for backward compatibility when working with older tooling.

## Overview

Morphir IR has two main format generations:

| Format | Versions | Characteristics |
|--------|----------|-----------------|
| **Classic** | V1, V2, V3 | Tagged array format: `["Variable", {}, ["name"]]` |
| **V4** | 4.x | Object wrapper format: `{ "Variable": { "name": "a" } }` |

The migrate command handles bidirectional conversion between these formats while preserving semantic meaning.

## Usage

```bash
morphir ir migrate --input <INPUT> --output <OUTPUT> [--target-version <VERSION>]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `-i, --input <INPUT>` | Input file path (morphir-ir.json) |
| `-o, --output <OUTPUT>` | Output file path for migrated IR |
| `--target-version <VERSION>` | Target format version: `v4` or `classic` (default: `v4`) |

## Examples

### Migrate Classic to V4

Convert an existing Classic format IR to the new V4 format:

```bash
morphir ir migrate \
    --input ./morphir-ir.json \
    --output ./morphir-ir-v4.json \
    --target-version v4
```

Output:
```
Migrating IR from "./morphir-ir.json" to "./morphir-ir-v4.json"
Converting Classic -> V4
Migration complete.
```

### Migrate V4 to Classic

Downgrade a V4 IR file to Classic format for compatibility with older tools:

```bash
morphir ir migrate \
    --input ./morphir-ir-v4.json \
    --output ./morphir-ir-classic.json \
    --target-version classic
```

Output:
```
Migrating IR from "./morphir-ir-v4.json" to "./morphir-ir-classic.json"
Converting V4 -> Classic
Migration complete.
```

### Default Behavior (Upgrade to V4)

When `--target-version` is omitted, the command defaults to V4:

```bash
morphir ir migrate \
    --input ./morphir-ir.json \
    --output ./morphir-ir-migrated.json
```

### In-place Migration

To migrate in-place (overwrite the original), use the same path for input and output:

```bash
morphir ir migrate \
    --input ./morphir-ir.json \
    --output ./morphir-ir.json \
    --target-version v4
```

## Format Differences

### Type Expressions

**Classic format:**
```json
["Variable", {}, ["a"]]
```

**V4 format:**
```json
{ "Variable": { "name": "a" } }
```

### Value Expressions

**Classic format:**
```json
["Apply", {},
  ["Reference", {}, [["morphir"], ["s", "d", "k"]], [["basics"]], ["add"]],
  ["Literal", {}, ["IntLiteral", 1]]
]
```

**V4 format:**
```json
{
  "Apply": {
    "function": {
      "Reference": { "fqName": "morphir/sdk:basics#add" }
    },
    "argument": {
      "Literal": { "value": { "IntLiteral": { "value": 1 } } }
    }
  }
}
```

### Module Structure

**Classic format:**
```json
{
  "formatVersion": 1,
  "distribution": ["Library", [...], [...], {
    "modules": [
      [["module", "name"], { "access": "Public", "value": {...} }]
    ]
  }]
}
```

**V4 format:**
```json
{
  "formatVersion": "4.0.0",
  "distribution": {
    "Library": {
      "packageName": "my/package",
      "dependencies": {},
      "def": {
        "modules": {
          "module/name": { "access": "Public", "value": {...} }
        }
      }
    }
  }
}
```

## Limitations

### V4 to Classic Downgrade

Some V4 constructs cannot be represented in Classic format:

- **Hole expressions**: Incomplete code placeholders
- **Native hints**: Platform-specific implementation details
- **External references**: Cross-platform bindings

Attempting to downgrade IR containing these constructs will result in an error.

### Metadata Loss

When converting between formats, some metadata may be lost or transformed:

| Classic → V4 | V4 → Classic |
|--------------|--------------|
| Empty `{}` attrs → TypeAttributes/ValueAttributes | Source locations lost |
| Array-based paths → Canonical strings | Constraints lost |
| Tuple entries → Keyed objects | Extensions preserved |

## Workflow Integration

### CI/CD Pipeline

Add migration as a build step to ensure V4 compatibility:

```yaml
# GitHub Actions example
- name: Migrate IR to V4
  run: |
    morphir ir migrate \
      --input ./morphir-ir.json \
      --output ./dist/morphir-ir.json \
      --target-version v4
```

### Pre-commit Hook

Validate and migrate on commit:

```bash
#!/bin/bash
# .git/hooks/pre-commit
morphir ir migrate \
    --input morphir-ir.json \
    --output morphir-ir.json \
    --target-version v4
git add morphir-ir.json
```

## Troubleshooting

### "Failed to load input"

Ensure the input file:
1. Exists at the specified path
2. Is valid JSON
3. Contains a valid Morphir IR structure

### "Cannot downgrade V4-only construct"

The IR contains V4-specific features. Either:
1. Remove the unsupported constructs from the source
2. Keep the IR in V4 format

### Version Detection

The command automatically detects the input format based on:
- `formatVersion` field (1-3 = Classic, 4+ = V4)
- Distribution structure (tagged array vs object wrapper)

## See Also

- [Morphir IR V4 Specification](./ir-v4-spec.md)
- [Format Migration Guide](./migration-guide.md)
- [API Reference](./api-reference.md)
