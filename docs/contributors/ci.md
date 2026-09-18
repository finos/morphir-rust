---
layout: default
title: Continuous Integration
nav_order: 5
parent: For Contributors
---

# Continuous integration

CI runs only the jobs a change can affect.

## How impact is computed

1. `changes` diffs the PR (or push) against its base and lists the changed paths.
2. Each path under `crates/<name>/` maps to that crate. `cargo metadata` supplies the
   workspace dependency graph, and every crate that depends on a changed crate
   (directly or transitively, including dev and build dependencies) is affected.
3. `.github/ci-impact.toml` declares the rest: paths that affect everything
   (`Cargo.lock`, `mise.toml`, the workflow itself), paths that affect nothing
   (`docs/`, `README.md`), and for each specialised job the crates and paths that
   turn it on.
4. Extension bundles run as a matrix over `.github/extensions.toml`, filtered to
   extensions whose crate is affected (or all of them when `morphir-daemon` is).

Anything the classifier does not recognise, and any classifier error, runs the
full suite.

## Forcing a full run

- Add the `ci:full` label to a pull request, then re-run the workflow (or push).
  The `changes` job reads labels live, so a re-run sees the new label.
- Trigger the workflow manually with `full = true`.
- A scheduled run every Monday runs everything.

## Caches and concurrency

- Rust jobs use `Swatinem/rust-cache` with three shared keys: `native`, `wasm`,
  and `coverage`. Only runs on `main` save; pull requests read main's cache.
- Superseded pull request runs are cancelled. Runs on `main` always finish.
- Every job has a timeout (15 minutes for quick jobs, 45 for build and test).

## Checking locally

    mise run ci:impact            # against origin/main
    mise run ci:impact main       # against another base
    mise run ci:impact --full     # what a full run would do

## Adding a crate or a job

- A new crate needs nothing: the graph picks it up.
- A new extension needs an entry in `.github/extensions.toml` and a
  `.mise/tasks/extension/artifact/<id>` task. The bundle matrix picks it up.
- A new specialised job needs a `[jobs.<name>]` entry in `ci-impact.toml`, an
  output line in the `changes` job, a gate on that output, and a line in `ci-ok`.
  `tests/ci/test_ci_impact_consistency.py` fails until all four are present.

## Required status check

Branch protection requires only `CI OK`. It fails when any needed job fails or is
cancelled, and passes when jobs were skipped.
