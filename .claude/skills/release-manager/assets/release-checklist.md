# Release Checklist for v{VERSION}

**Release Manager**: ________________
**Date**: ________________

## Pre-Release

- [ ] All tests passing: `mise run release:check`
- [ ] No uncommitted changes: `git status`
- [ ] On correct branch (main or release branch)
- [ ] CHANGELOG.md has entries in [Unreleased]
- [ ] CHANGELOG.md validates: `mise run release:changelog-validate`
- [ ] Documentation updated for new features
- [ ] README accurate for new features
- [ ] Breaking changes documented (if any)

## Version & Changelog

- [ ] Version decided: `_________` (MAJOR/MINOR/PATCH)
- [ ] Version bumped: `mise run release:version-bump {VERSION}`
- [ ] Cargo.lock updated: `cargo check`
- [ ] CHANGELOG.md updated:
  - [ ] New version section added with date
  - [ ] Entries moved from [Unreleased]
  - [ ] [Unreleased] section cleared
  - [ ] Comparison links updated

## Commit & Tag

- [ ] Changes committed: `git commit -m "chore: prepare release v{VERSION}"`
- [ ] Tag created: `mise run release:tag-create {VERSION}`
- [ ] Tag pushed: `git push origin v{VERSION}`

## CI/CD

- [ ] GitHub Actions workflow started
- [ ] All matrix builds passing:
  - [ ] x86_64-unknown-linux-gnu
  - [ ] x86_64-unknown-linux-musl
  - [ ] aarch64-unknown-linux-gnu
  - [ ] x86_64-apple-darwin
  - [ ] aarch64-apple-darwin
  - [ ] x86_64-pc-windows-msvc
  - [ ] aarch64-pc-windows-msvc
- [ ] Draft release created
- [ ] All 7 artifacts uploaded

## Release Review

- [ ] Release notes reviewed
- [ ] Highlights added (if needed)
- [ ] Breaking changes noted (if any)
- [ ] Upgrade instructions included (if needed)

## Publish

- [ ] **Release published** (no longer draft)
- [ ] Release URL: https://github.com/finos/morphir-rust/releases/tag/v{VERSION}

## Post-Release

- [ ] Artifacts downloadable: `mise run release:post-release {VERSION}`
- [ ] `cargo binstall morphir` works
- [ ] Documentation site updated (if applicable)
- [ ] Announcements posted:
  - [ ] ________________
  - [ ] ________________

## Issues Encountered

<!--
Document any issues found during release:

| Issue | Resolution | Follow-up |
|-------|------------|-----------|
| ... | ... | Issue #XX |
-->

## Notes

<!--
Any additional notes about this release:
-->
