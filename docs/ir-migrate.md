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
morphir ir migrate --input <INPUT> --output <OUTPUT> [OPTIONS]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `-i, --input <INPUT>` | Input source: local file path, URL, or remote source shorthand |
| `-o, --output <OUTPUT>` | Output file path for migrated IR |
| `--target-version <VERSION>` | Target format version: `v4` or `classic` (default: `v4`) |
| `--force-refresh` | Force refresh cached remote sources |
| `--no-cache` | Skip cache entirely for remote sources |

### Supported Input Sources

The `--input` argument accepts multiple source types:

| Source Type | Format | Example |
|-------------|--------|---------|
| **Local file** | File path | `./morphir-ir.json` |
| **HTTP/HTTPS** | URL | `https://example.com/morphir-ir.json` |
| **GitHub shorthand** | `github:owner/repo[@ref][/path]` | `github:finos/morphir-examples@main/examples/basic` |
| **Git URL** | `https://*.git` | `https://github.com/org/repo.git` |
| **Gist** | `gist:id[#filename]` | `gist:abc123#morphir-ir.json` |

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

## Remote Sources

The migrate command supports fetching IR from remote sources, making it easy to work with published Morphir models without downloading them manually.

### HTTP/HTTPS URLs

Fetch and migrate IR directly from a URL:

```bash
# Migrate the LCR (Liquidity Coverage Ratio) IR - a comprehensive regulatory model
morphir ir migrate \
    --input https://lcr-interactive.finos.org/server/morphir-ir.json \
    --output ./lcr-v4.json \
    --target-version v4
```

The LCR IR is a large, comprehensive example of Morphir in production use, implementing the Basel III Liquidity Coverage Ratio regulation.

### GitHub Shorthand

Use the `github:` shorthand for GitHub repositories:

```bash
# Fetch from a specific branch
morphir ir migrate \
    --input github:finos/morphir-examples@main/examples/basic/morphir-ir.json \
    --output ./example-v4.json

# Fetch from a tag
morphir ir migrate \
    --input github:finos/morphir-examples@v1.0.0/examples/basic/morphir-ir.json \
    --output ./example-v4.json
```

### Caching

Remote sources are cached locally to avoid repeated downloads:

```bash
# Force refresh - re-download even if cached
morphir ir migrate \
    --input https://lcr-interactive.finos.org/server/morphir-ir.json \
    --output ./lcr-v4.json \
    --force-refresh

# Skip cache entirely
morphir ir migrate \
    --input github:finos/morphir-examples/examples/basic/morphir-ir.json \
    --output ./example-v4.json \
    --no-cache
```

Cache is stored in `~/.cache/morphir/sources/` by default.

### Configuration

Remote source access can be configured in `morphir.toml`:

```toml
[sources]
enabled = true
allow = ["github:finos/*", "https://lcr-interactive.finos.org/*"]
deny = ["*://untrusted.com/*"]
trusted_github_orgs = ["finos", "morphir-org"]

[sources.cache]
directory = "~/.cache/morphir/sources"
max_size_mb = 500
ttl_secs = 86400  # 24 hours
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

### "Source URL not allowed by configuration"

The remote source is blocked by your `morphir.toml` configuration:
1. Check the `[sources.allow]` and `[sources.deny]` lists
2. Add the source URL to your allow list
3. Or use `--no-cache` to bypass configuration checks

### "Failed to fetch source"

For remote source errors:
1. Check your network connection
2. Verify the URL is correct and accessible
3. For GitHub sources, ensure the repository is public or you have access
4. Try `--force-refresh` to bypass potentially corrupted cache
5. Check if git is installed (required for `github:` sources)

### Version Detection

The command automatically detects the input format based on:
- `formatVersion` field (1-3 = Classic, 4+ = V4)
- Distribution structure (tagged array vs object wrapper)

## See Also

- [Morphir IR V4 Specification](./ir-v4-spec.md)
- [Format Migration Guide](./migration-guide.md)
- [API Reference](./api-reference.md)
