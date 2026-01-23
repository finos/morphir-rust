# Version Policy

morphir-rust follows [Semantic Versioning 2.0.0](https://semver.org/).

## Version Format

```
MAJOR.MINOR.PATCH[-PRERELEASE]

Examples:
  0.1.0
  0.2.0-beta.1
  1.0.0
  1.2.3
```

## When to Bump

### MAJOR (X.0.0)

Increment for **incompatible API changes**:

- Removing public functions, types, or modules
- Changing function signatures in breaking ways
- Changing CLI command syntax incompatibly
- Removing CLI commands or flags
- Changing default behavior in breaking ways
- Major restructuring that affects users

**Examples**:
- Removing `morphir validate` command
- Changing `--output` flag to require a value when it was optional
- Renaming `MorphirIR` type to `IR`

### MINOR (0.X.0)

Increment for **new functionality** (backward compatible):

- Adding new CLI commands
- Adding new flags to existing commands
- Adding new features
- Adding new public types or functions
- Deprecating functionality (not removing)
- Significant performance improvements

**Examples**:
- Adding `morphir analyze` command
- Adding `--verbose` flag to `morphir validate`
- Adding support for a new IR version

### PATCH (0.0.X)

Increment for **backward compatible bug fixes**:

- Bug fixes that don't change the API
- Security patches
- Documentation corrections
- Minor performance improvements
- Internal refactoring (no external impact)

**Examples**:
- Fixing crash when parsing malformed IR
- Fixing incorrect error message
- Improving validation error details

## Pre-Release Versions

For testing before stable release:

```
MAJOR.MINOR.PATCH-PRERELEASE

Examples:
  0.2.0-alpha.1   # Early testing, unstable
  0.2.0-beta.1    # Feature complete, testing
  0.2.0-rc.1      # Release candidate, final testing
```

Pre-release versions:
- Have lower precedence than the release version
- Should not be used in production
- May have breaking changes between pre-releases

## 0.x.y Versions (Current State)

While the major version is 0 (e.g., 0.1.0):

- The API is considered **unstable**
- MINOR bumps (0.x.0) **may** include breaking changes
- Moving to 1.0.0 signals **API stability commitment**

This allows rapid iteration during initial development.

## Decision Guide

```
Did you remove or change existing public API?
  YES → MAJOR bump
  NO  ↓

Did you add new features or functionality?
  YES → MINOR bump
  NO  ↓

Did you fix bugs without changing the API?
  YES → PATCH bump
  NO  → No version bump needed (docs, CI, etc.)
```

## Commit Message Convention

Use conventional commits to help determine version bumps:

| Prefix | Version Impact |
|--------|----------------|
| `feat:` | MINOR bump |
| `fix:` | PATCH bump |
| `feat!:` or `BREAKING CHANGE:` | MAJOR bump |
| `chore:`, `docs:`, `ci:` | No bump |

## Examples from morphir-rust

| Change | Version Bump |
|--------|--------------|
| Add `morphir analyze` command | MINOR |
| Fix crash in `morphir validate` | PATCH |
| Remove deprecated `--legacy` flag | MAJOR |
| Add `--format json` to `morphir schema` | MINOR |
| Improve error messages | PATCH |
| Update dependencies | PATCH (usually) |
| Add new IR version support | MINOR |
| Change IR format incompatibly | MAJOR |
