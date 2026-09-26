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

| Crate | Current version | Depends on |
|-------|-----------------|------------|
| `morphir-config` | `0.0.1` | no workspace crates |
| `morphir-core` | `0.2.0` | no workspace crates |
| `morphir-workspace` | `0.2.0` | `morphir-config` |
| `morphir-extension-sdk` | `0.2.0` | `morphir-workspace` |
| `morphir-projection` | `0.1.0` | `morphir-core` |

The other crates in the workspace are not on crates.io. They keep the shared
workspace version (`[workspace.package] version` in the root `Cargo.toml`).

A crate that goes to crates.io leaves the shared version. Its `Cargo.toml`
sets `version` explicitly, for example `version = "0.2.0"`.

## Version rule

Published crates use `0.x.y` versions until Morphir itself reaches 1.0.0.

- Increase `x` for a breaking change. Set `y` to 0.
- Increase `y` for an additive change or a fix.
- Prereleases are allowed, for example `0.2.0-alpha.1`.
- Do not publish `1.0.0` or higher before Morphir 1.0.0.

Cargo treats `0.x` as the compatibility range, so a change to `x` tells
users that the change can break them.

## Publish order

A published crate can only depend on other published crates. Every path
dependency on a published crate also gives a `version`, anywhere in the
workspace:

```toml
morphir-workspace = { path = "../morphir-workspace", version = "0.2.0" }
```

A workspace build uses the path. The packaged crate depends on that version
from crates.io, so that version must be on crates.io before the crate that
depends on it is published. When you change the version of a published crate,
change the `version` of every path dependency on it too.

Publish the crates in tiers:

- Tier 0: `morphir-config`, which is already on crates.io.
- Tier 1: `morphir-core` and `morphir-workspace`.
- Tier 2: `morphir-extension-sdk` and `morphir-projection`.

Publish a tier only after all the crates of the tier before it are visible on
crates.io. The crates of one tier do not depend on each other, so you can
publish them in any order.

## Packaging check in CI

The `Package (published crates)` job in the CI workflow packages all the
published crates together:

```bash
cargo package -p morphir-config -p morphir-core -p morphir-workspace \
  -p morphir-extension-sdk -p morphir-projection --locked
```

Cargo builds each package against the packages of the workspace crates it
depends on, not against the workspace sources. This finds a crate that would
not build from crates.io, before its dependencies are published. The job runs
when one of these crates, `Cargo.toml` or `Cargo.lock` changes.

Some crates leave their tests out of the package with `exclude = ["/tests/"]`,
because the tests read shared fixtures from the workspace or need a path
dev-dependency that the package drops.

## How to publish a crate

1. Set the new version in `crates/<crate>/Cargo.toml` and run `cargo check`
   so that `Cargo.lock` agrees.
2. Add the change to `CHANGELOG.md`.
3. Make sure that the dry run passes:

   ```bash
   cargo publish --dry-run -p <crate> --locked
   ```

   The dry run gets the dependencies of the crate from crates.io. Before the
   tier below is published, use the packaging check above instead.

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
