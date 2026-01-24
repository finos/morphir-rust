---
layout: default
title: morphir self
parent: CLI Reference
nav_order: 7
---

# `morphir self`

**Usage**: `morphir self <COMMAND>`

Manage the morphir installation, including version management and dev mode.

The `self` command provides utilities for managing your morphir installation, switching between versions, and enabling development mode.

## Commands

### `morphir self upgrade`

Download and install the latest version of morphir.

```bash
morphir self upgrade
```

This clears the version cache and fetches the latest release from GitHub.

### `morphir self list`

List all installed versions of morphir.

```bash
morphir self list
```

**Example output:**

```
Installed versions:
  0.1.0
  0.2.0
```

### `morphir self which`

Show which version of morphir would be used based on current configuration.

```bash
morphir self which
```

**Example output:**

```
Version: 0.1.0
Binary: /home/user/.morphir/versions/0.1.0/morphir-bin
Status: installed
Backend: github
```

### `morphir self install <VERSION>`

Install a specific version of morphir.

```bash
morphir self install 0.1.0
```

### `morphir self prune`

Remove old versions of morphir, keeping only the currently active version.

```bash
morphir self prune
```

### `morphir self update`

Update the morphir launcher script itself (not the morphir binary).

```bash
morphir self update
```

This downloads the latest launcher script from the repository.

### `morphir self dev`

Show dev mode status and configuration.

```bash
morphir self dev
```

**Example output:**

```
info: Dev mode status:

  MORPHIR_DEV env:     not set
  .morphir-version:    not found
  morphir.toml:        dev_mode not set
  MORPHIR_DEV_PATH:    not set (will auto-detect)

  Source directory:    /home/user/code/morphir-rust
  Debug binary:        /home/user/code/morphir-rust/target/debug/morphir (available)
  Release binary:      not built

Dev mode is DISABLED

To enable dev mode, use one of:
  - morphir --dev <command>        (one-time)
  - export MORPHIR_DEV=1           (session)
  - echo 'local-dev' > .morphir-version  (project)
  - Add 'dev_mode = true' to morphir.toml [morphir] section
```

## Version Override

Run morphir with a specific version using the `+` syntax:

```bash
morphir +0.1.0 <command>
```

This downloads and uses the specified version without changing your default configuration.

## Dev Mode Flag

Run morphir from local source using the `--dev` flag:

```bash
morphir --dev <command>
```

This builds and runs morphir from a local source checkout. See [Dev Mode](#dev-mode) for details on how the source directory is detected.

## Dev Mode

Dev mode runs morphir from a local source checkout instead of a downloaded binary.

### Enabling Dev Mode

| Method | Scope | Example |
|--------|-------|---------|
| `--dev` flag | One command | `morphir --dev ir migrate ...` |
| `MORPHIR_DEV=1` | Session | `export MORPHIR_DEV=1` |
| `.morphir-version` | Project | `echo "local-dev" > .morphir-version` |
| `morphir.toml` | Project | `dev_mode = true` in `[morphir]` section |

### Source Detection

When dev mode is enabled, the source directory is found by checking:

1. `MORPHIR_DEV_PATH` environment variable
2. CI environment variables (GitHub Actions, GitLab CI, Jenkins, etc.)
3. Current directory and parent directories (walks up to find `Cargo.toml` with workspace)
4. Common development locations:
   - `~/code/morphir-rust`
   - `~/dev/morphir-rust`
   - `~/src/morphir-rust`
   - `~/projects/morphir-rust`

### Build Behavior

In dev mode:

1. If a debug binary exists and is newer than source files, it's used directly
2. Otherwise, `cargo run` is invoked to build and run

## Environment Variables

| Variable | Description |
|----------|-------------|
| `MORPHIR_VERSION` | Override version to use |
| `MORPHIR_HOME` | Override home directory (default: `~/.morphir`) |
| `MORPHIR_BACKEND` | Force backend: `mise`, `binstall`, `github`, `cargo` |
| `MORPHIR_DEV` | Set to `1` to enable dev mode |
| `MORPHIR_DEV_PATH` | Path to morphir-rust source directory |

## Installation Backends

The launcher supports multiple backends for downloading morphir:

| Backend | Description |
|---------|-------------|
| `mise` | Uses [mise](https://mise.jdx.dev/) version manager (preferred if available) |
| `binstall` | Uses [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) for pre-built binaries |
| `github` | Downloads directly from GitHub Releases (default fallback) |
| `cargo` | Compiles from source using `cargo install` |

Backend is auto-detected based on available tools, or can be forced with `MORPHIR_BACKEND`.

## See Also

- [Installation](../install) - Installation guide with detailed options
- [Getting Started](../getting-started) - Basic usage guide
