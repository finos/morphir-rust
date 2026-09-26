---
layout: default
title: Publishing Crates
nav_order: 7
parent: For Contributors
---

# Publishing crates to crates.io

Some library crates in this workspace go to [crates.io](https://crates.io).
Each published crate has its own version and its own release tag. A crate tag
starts the `Publish crate` workflow (`.github/workflows/publish-crate.yml`),
which publishes that one crate.

## Which crates are published

| Crate | First version | Status |
|-------|---------------|--------|
| `morphir-config` | `0.0.1` | First crate. It proves the publish pipeline. |

The other crates in the workspace are not on crates.io. They keep the shared
workspace version (`[workspace.package] version` in the root `Cargo.toml`).

A crate that goes to crates.io leaves the shared version. Its `Cargo.toml`
sets `version` explicitly, for example `version = "0.0.1"`.

## Version rule

Published crates use `0.x.y` versions until Morphir itself reaches 1.0.0.

- Increase `x` for a breaking change. Set `y` to 0.
- Increase `y` for an additive change or a fix.
- Prereleases are allowed, for example `0.2.0-alpha.1`.
- Do not publish `1.0.0` or higher before Morphir 1.0.0.

Cargo treats `0.x` as the compatibility range, so a change to `x` tells
users that the change can break them.

## Publish order

A published crate can only depend on other published crates. A path
dependency on a workspace crate must also give a `version`, and that version
must already be on crates.io. Publish crates in this order:

1. `morphir-config`, which has no workspace dependencies.
2. `morphir-core` and `morphir-workspace`.
3. `morphir-common`, which depends on the crates above.

Before you publish a crate that depends on another workspace crate, change
the dependency to `{ path = "../<crate>", version = "<published version>" }`.

## How to publish a crate

1. Set the new version in `crates/<crate>/Cargo.toml` and run `cargo check`
   so that `Cargo.lock` agrees.
2. Add the change to `CHANGELOG.md`.
3. Make sure that the dry run passes:

   ```bash
   cargo publish --dry-run -p <crate> --locked
   ```

4. Merge the change to `main`.
5. Tag the merge commit on `main` and push the tag:

   ```bash
   git tag -a crates/<crate>/v<version> -m "<crate> <version>" <commit>
   git push origin crates/<crate>/v<version>
   ```

   For example, `crates/morphir-config/v0.0.1`.

To run the workflow again for a tag that already exists, start
`Publish crate` from the Actions tab and give the tag name as input.

The `v*` and `extension/*/v*` tags start the extension release workflow.
Crate tags do not match those patterns, so a crate tag starts only the
crate publish workflow.

## What the workflow checks

The `resolve` job runs `.github/scripts/crate_release.py`. It stops the
workflow if:

- the tag is not `crates/<crate>/v<semver>`,
- there is no crate at `crates/<crate>`, or its package name is different,
- the crate sets `publish = false`,
- the version in the crate's `Cargo.toml` is not the tag version,
- the tag commit is not on `main`.

The `publish` job checks out the tag commit, runs
`cargo publish --dry-run -p <crate> --locked` and then
`cargo publish -p <crate> --locked`.

## Credentials

The workflow uses the `CARGO_REGISTRY_TOKEN` repository secret. crates.io
needs a token for the first publish of a crate, because trusted publishing
can only be set up for a crate that already exists.

Follow-up: after the first publish of a crate, set up trusted publishing for
it on crates.io. Then change the workflow to get a short-lived token from
`rust-lang/crates-io-auth-action` in a `crates.io` environment (with
`id-token: write`), and stop using the long-lived token.

## If a publish fails

crates.io does not let you replace a version that is already published.

- If the `resolve` job fails, nothing was published. Correct the cause. If
  the tag is wrong, delete it and tag again.
- If the `publish` job fails before the upload, nothing was published. Fix
  the cause and run the workflow again for the same tag.
- If a published version is bad, publish a new version with the fix. Use
  `cargo yank --version <version> <crate>` to stop new projects from
  selecting the bad version.
