# Release Workflow

Complete step-by-step process for releasing morphir-rust.

## Overview

morphir-rust uses **tag-triggered releases** via GitHub Actions. Pushing a `v*` tag triggers the release workflow which builds binaries for all platforms and creates a draft GitHub release.

## Prerequisites

- Clean git working directory
- All tests passing
- CHANGELOG.md updated
- On the main branch (or release branch)

## Complete Workflow

### Phase 1: Preparation

#### Step 1: Ensure clean state
```bash
git checkout main
git pull origin main
git status  # Should show "nothing to commit, working tree clean"
```

#### Step 2: Run pre-release checks
```bash
mise run release:pre-release <version>
```

This validates:
- Code formatting (cargo fmt)
- Linting (clippy)
- Tests pass
- Release build succeeds
- CHANGELOG.md format is valid

#### Step 3: Review changelog
```bash
# View unreleased changes
head -50 CHANGELOG.md
```

Ensure all significant changes are documented.

#### Step 4: Determine version
Based on changes since last release:
- **MAJOR** (X.0.0): Breaking API changes
- **MINOR** (0.X.0): New features, backward compatible
- **PATCH** (0.0.X): Bug fixes only

### Phase 2: Version & Changelog

#### Step 5: Bump version
```bash
mise run release:version-bump <version>
cargo check  # Updates Cargo.lock
```

#### Step 6: Update changelog
```bash
mise run release:changelog-release <version>
```

Then manually in `CHANGELOG.md`:
1. Add `## [<version>] - YYYY-MM-DD` after empty Unreleased categories
2. Move all entries from `[Unreleased]` to the new version section
3. Clear `[Unreleased]` categories (keep the headers)
4. Update comparison links at bottom

### Phase 3: Commit & Tag

#### Step 7: Commit release preparation
```bash
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore: prepare release v<version>"
```

#### Step 8: Create and push tag
```bash
mise run release:tag-create <version> --push
```

This creates an annotated tag and pushes it, triggering the release workflow.

### Phase 4: GitHub Actions

#### Step 9: Monitor release workflow
- Go to: https://github.com/finos/morphir-rust/actions
- Watch the "Release" workflow
- Wait for all 7 matrix builds to complete

The workflow builds for:
- `x86_64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

#### Step 10: Review draft release
- Go to: https://github.com/finos/morphir-rust/releases
- Click on the draft release
- Verify all 7 artifacts are uploaded
- Review auto-generated release notes
- Edit description if needed (add highlights, breaking changes)

#### Step 11: Publish release
- Click "Publish release"
- The release is now live

### Phase 5: Post-Release

#### Step 12: Validate
```bash
mise run release:post-release <version>
```

Verify:
- All artifacts downloadable
- `cargo binstall morphir` works
- Documentation is current

#### Step 13: Announce (if applicable)
- Update project documentation
- Post to relevant channels
- Update any dependent projects

## Quick Reference

```bash
# Full release flow (after ensuring clean state)
mise run release:pre-release 0.2.0
mise run release:version-bump 0.2.0
# Edit CHANGELOG.md manually
cargo check
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore: prepare release v0.2.0"
mise run release:tag-create 0.2.0 --push
# Wait for CI, review draft, publish
mise run release:post-release 0.2.0
```

## Rollback Procedure

If a release has issues after tagging but before publishing:

```bash
# Delete local tag
git tag -d v<version>

# Delete remote tag
git push origin :refs/tags/v<version>

# Delete draft release on GitHub (if created)
gh release delete v<version> --yes

# Fix the issue, then start over
```

If already published, create a new patch release instead.
