# Release Troubleshooting

Common issues and solutions for the morphir-rust release process.

## Tag Issues

### Tag Already Exists

**Symptom**: `mise run release:tag-create` fails with "tag already exists"

**Solution**:
```bash
# If tag was NOT pushed yet (local only)
git tag -d v<version>
# Then try again

# If tag WAS pushed to remote
git tag -d v<version>
git push origin :refs/tags/v<version>
# Then try again
```

### Wrong Commit Tagged

**Symptom**: Tag points to wrong commit

**Solution**:
```bash
# Delete and recreate
git tag -d v<version>
git push origin :refs/tags/v<version>

# Make sure you're on the correct commit
git log --oneline -5

# Recreate tag
mise run release:tag-create <version> --push
```

## CI/CD Issues

### Build Fails for Some Targets

**Symptom**: Release workflow shows failed jobs

**Common causes**:
1. Cross-compilation issues
2. Platform-specific code bugs
3. Missing system dependencies

**Solution**:
1. Check the failed job logs in GitHub Actions
2. Reproduce locally if possible:
   ```bash
   # For Linux targets
   cargo build --release --target x86_64-unknown-linux-gnu

   # For cross-compilation (requires cross or appropriate toolchain)
   cross build --release --target aarch64-unknown-linux-gnu
   ```
3. Fix the issue and re-tag

### Release Not Created

**Symptom**: Tag pushed but no draft release appears

**Possible causes**:
- `build-release` job failed, so `publish-release` didn't run
- GitHub Actions permissions issue

**Solution**:
1. Check all matrix jobs completed successfully
2. If builds succeeded but release wasn't created:
   ```bash
   # Create release manually
   gh release create v<version> --draft --generate-notes

   # Download artifacts from workflow run
   # Then upload manually
   gh release upload v<version> ./artifacts/*
   ```

### Permissions Error

**Symptom**: "Resource not accessible by integration"

**Solution**:
1. Check repository Settings → Actions → General
2. Ensure "Read and write permissions" is enabled
3. Re-run the workflow

## Changelog Issues

### Validation Fails

**Symptom**: `mise run release:changelog-validate` reports errors

**Common fixes**:
```bash
# Missing [Unreleased] section
# Add at top after header

# Missing comparison links
# Add at bottom of file:
[Unreleased]: https://github.com/finos/morphir-rust/compare/v<latest>...HEAD
```

### Forgot to Update Changelog

**Symptom**: Released but changelog still shows entries in [Unreleased]

**Solution** (before publishing release):
1. Update CHANGELOG.md
2. Amend the release commit:
   ```bash
   git add CHANGELOG.md
   git commit --amend --no-edit
   ```
3. Force update the tag:
   ```bash
   git tag -fa v<version> -m "Release <version>"
   git push origin v<version> --force
   ```
4. Wait for CI to rebuild

## Artifact Issues

### Missing Artifacts

**Symptom**: Not all 7 artifacts in release

**Expected artifacts**:
- morphir-VERSION-x86_64-unknown-linux-gnu.tgz
- morphir-VERSION-x86_64-unknown-linux-musl.tgz
- morphir-VERSION-aarch64-unknown-linux-gnu.tgz
- morphir-VERSION-x86_64-apple-darwin.tgz
- morphir-VERSION-aarch64-apple-darwin.tgz
- morphir-VERSION-x86_64-pc-windows-msvc.zip
- morphir-VERSION-aarch64-pc-windows-msvc.zip

**Solution**:
1. Check which matrix jobs failed
2. Re-run failed jobs in GitHub Actions, or
3. Upload missing artifacts manually

### Artifact Corrupted

**Symptom**: Downloaded artifact doesn't work or extract

**Solution**:
1. Delete the corrupted artifact from release
2. Rebuild locally:
   ```bash
   cargo build --release --target <target>
   # Archive appropriately
   tar -czvf morphir-<version>-<target>.tgz -C target/<target>/release morphir
   ```
3. Upload manually:
   ```bash
   gh release upload v<version> morphir-<version>-<target>.tgz
   ```

## Version Issues

### Version Mismatch

**Symptom**: Tag version doesn't match Cargo.toml version

**Solution**:
```bash
# Fix Cargo.toml
mise run release:version-bump <correct-version>
cargo check  # Update Cargo.lock

# Amend commit
git add Cargo.toml Cargo.lock
git commit --amend --no-edit

# Update tag
git tag -d v<version>
git tag -a v<version> -m "Release <version>"
git push origin v<version> --force
```

### Published Wrong Version

**Symptom**: Release is live but has issues

**Solution**: You cannot unpublish GitHub releases safely (users may have downloaded). Instead:
1. Create a new patch release with fixes
2. Add note to the problematic release describing the issue
3. Consider yanking from crates.io if published there

## binstall Issues

### binstall Can't Find Package

**Symptom**: `cargo binstall morphir` fails

**Possible causes**:
1. Release not published (still draft)
2. Artifact naming doesn't match binstall expectations
3. Package not on crates.io

**Solution**:
1. Ensure release is published (not draft)
2. Verify artifact names match pattern in `Cargo.toml`:
   ```
   morphir-<version>-<target>.tgz
   morphir-<version>-<target>.zip (Windows)
   ```
3. For crates.io: `cargo publish` separately

## Getting Help

If you encounter an issue not covered here:

1. Check GitHub Actions logs for detailed errors
2. Search existing issues: https://github.com/finos/morphir-rust/issues
3. Create a new issue with:
   - Release version attempted
   - Error messages
   - Steps to reproduce
   - Relevant logs
