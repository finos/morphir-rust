---
layout: default
title: Complete Workflow
nav_order: 4
parent: Tutorials
---

# Complete Workflow

This guide walks through a complete end-to-end workflow using Morphir with Gleam.

## Scenario: Building a Business Logic Library

Let's build a simple business logic library for a financial application.

## Step 1: Project Setup

```bash
mkdir finance-library
cd finance-library
mkdir src
```

Create `morphir.toml`:

```toml
[project]
name = "Finance.Library"
version = "0.1.0"
description = "Financial calculations library"
source_directory = "src"
```

## Step 2: Write Business Logic

Create `src/calculations.gleam`:

```gleam
pub fn calculate_interest(principal: Float, rate: Float, years: Int) -> Float {
  principal * (1.0 + rate) ** float(years)
}

pub fn calculate_payment(principal: Float, rate: Float, periods: Int) -> Float {
  let monthly_rate = rate / 12.0
  principal * (monthly_rate * (1.0 + monthly_rate) ** float(periods)) / 
    ((1.0 + monthly_rate) ** float(periods) - 1.0)
}
```

Create `src/types.gleam`:

```gleam
pub type Account {
  Account(account_number: String, balance: Float, interest_rate: Float)
}

pub fn get_balance(account: Account) -> Float {
  case account {
    Account(_, balance, _) -> balance
  }
}
```

## Step 3: Compile to Morphir IR

```bash
morphir gleam compile
```

Output:
```
Compilation successful!
Output: .morphir/out/Finance.Library/compile/gleam/
```

Inspect the IR:

```bash
cat .morphir/out/Finance.Library/compile/gleam/format.json
```

## Step 4: Verify IR Structure

The IR is stored as a document tree:

```
.morphir/out/Finance.Library/compile/gleam/
├── format.json
└── modules/
    └── Finance.Library/
        ├── calculations/
        │   ├── module.json
        │   └── values/
        │       ├── calculate_interest.json
        │       └── calculate_payment.json
        └── types/
            ├── module.json
            ├── types/
            │   └── Account.json
            └── values/
                └── get_balance.json
```

## Step 5: Generate Code (Roundtrip)

Generate Gleam code from the IR:

```bash
morphir gleam generate
```

This creates:

```
.morphir/out/Finance.Library/generate/gleam/
├── calculations.gleam
└── types.gleam
```

## Step 6: Verify Roundtrip

Compare original and generated code:

```bash
# Original
cat src/calculations.gleam

# Generated
cat .morphir/out/Finance.Library/generate/gleam/calculations.gleam
```

The generated code should be semantically equivalent.

## Step 7: Use JSON Output for Automation

For CI/CD or automation:

```bash
morphir gleam compile --json > compile-result.json
```

Parse the result:

```bash
jq '.success' compile-result.json
jq '.modules[]' compile-result.json
```

## Step 8: Workspace Setup (Optional)

If you have multiple projects, set up a workspace:

```toml
# morphir.toml (workspace root)
[workspace]
members = ["finance-library", "reporting-service"]
default_member = "finance-library"

[project]
name = "workspace"
```

Compile a specific project:

```bash
morphir gleam compile --project finance-library
```

## Step 9: Integration with Build System

### Makefile Example

```makefile
.PHONY: compile generate roundtrip

compile:
	morphir gleam compile

generate:
	morphir gleam generate

roundtrip: compile generate
	@echo "Roundtrip complete"

test: roundtrip
	@echo "Comparing original and generated code..."
	diff -r src/ .morphir/out/Finance.Library/generate/gleam/ || true
```

### CI/CD Example (GitHub Actions)

```yaml
name: Morphir Build

on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Install Morphir
        run: curl -fsSL https://raw.githubusercontent.com/finos/morphir-rust/main/scripts/install.sh | bash
      - name: Compile to IR
        run: morphir gleam compile --json > compile-result.json
      - name: Check compilation
        run: |
          if [ "$(jq -r '.success' compile-result.json)" != "true" ]; then
            echo "Compilation failed"
            exit 1
          fi
      - name: Generate code
        run: morphir gleam generate
      - name: Upload artifacts
        uses: actions/upload-artifact@v3
        with:
          name: morphir-ir
          path: .morphir/out/
```

## Step 10: Debugging

### Enable Verbose Output

```bash
RUST_LOG=debug morphir gleam compile
```

### Check Diagnostics

```bash
morphir gleam compile --json | jq '.diagnostics'
```

### View Error Details

Errors are reported with source locations:

```
Error: Compilation failed
  ┌─ src/calculations.gleam:3:5
  │
3 │   principal * (1.0 + rate) ** float(years)
  │   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  │   Type mismatch: expected Float, found Int
```

## Best Practices

1. **Version Control**: Add `.morphir/out/` to `.gitignore`, but keep `.morphir/test/` versioned
2. **Configuration**: Use `morphir.toml` for project settings
3. **Roundtrip Testing**: Regularly run roundtrip to verify semantic preservation
4. **JSON Output**: Use `--json` for automation and CI/CD
5. **Workspaces**: Use workspaces for multi-project setups

## Troubleshooting

### Common Issues

**Issue**: "No morphir.toml found"
- **Solution**: Create `morphir.toml` in project root or use `--config`

**Issue**: "Extension not found"
- **Solution**: Ensure Gleam binding is built and bundled

**Issue**: "Path does not exist"
- **Solution**: Check `source_directory` in config matches your project structure

## Next Steps

- Explore [Extension Development](../contributors/extension-tutorial)
- Read [Architecture Documentation](../contributors/architecture)
- Check [CLI Reference](../cli/index)
