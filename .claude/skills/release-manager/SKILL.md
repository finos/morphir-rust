---
name: release-manager
description: Manages release workflows for morphir-rust. Use when preparing releases, bumping versions, updating changelogs, creating tags, validating pre-release requirements, performing post-release retrospectives, generating release summaries, or troubleshooting release issues. Invoked with /release-manager [mode] [args].
---

# Release Manager Skill

You are a release management assistant for the morphir-rust project. Help users with the complete release lifecycle.

## Modes of Operation

### 1. Prepare Mode
**Invocation**: `/release-manager prepare [version]`

Guide through pre-release preparation:

1. **Analyze current state**:
   - Run `git status` to check for uncommitted changes
   - Check current version in `Cargo.toml`
   - Run `mise run release:changelog-validate` to validate changelog
   - Review unreleased changes in `CHANGELOG.md`

2. **Run quality checks**:
   - Suggest running `mise run release:pre-release <version>`
   - This runs tests, clippy, build, and changelog validation

3. **Guide version decision**:
   - Review commits since last tag to determine bump type
   - MAJOR: Breaking API changes
   - MINOR: New features, backward compatible
   - PATCH: Bug fixes only

4. **Execute preparation steps**:
   ```bash
   mise run release:version-bump <version>
   mise run release:changelog-release <version>
   # Help user move entries from [Unreleased] to [version]
   cargo check  # Update Cargo.lock
   mise run docs:generate  # Update docs site (CLI docs + release notes)
   git add Cargo.toml Cargo.lock CHANGELOG.md docs/
   git commit -m "chore: prepare release v<version>"
   mise run release:tag-create <version> --push
   ```

### 2. Analyze Mode
**Invocation**: `/release-manager analyze`

Analyze the repository state for release readiness:

1. **Commit analysis**:
   - Parse commits since last tag using `git log`
   - Categorize by conventional commit type (feat, fix, chore, etc.)
   - Identify breaking changes (look for "BREAKING" in messages)
   - Generate suggested changelog entries

2. **Dependency check**:
   - Look for recent dependency updates in commits
   - Note any security-related updates

3. **Documentation review**:
   - Check if README needs updates for new features
   - Verify CHANGELOG has entries for significant changes

4. **Output format**:
   ```
   ## Release Analysis

   ### Commits since last release
   - feat: ... (suggest for Added)
   - fix: ... (suggest for Fixed)

   ### Suggested changelog entries
   ### Added
   - ...
   ### Fixed
   - ...

   ### Recommendations
   - Suggested version bump: MINOR/PATCH
   - Documentation updates needed: Yes/No
   ```

### 3. Retrospective Mode
**Invocation**: `/release-manager retrospective [version]`

Post-release validation and issue tracking:

1. **Validate release**:
   - Run `mise run release:post-release <version>`
   - Check tag exists locally and on remote
   - Verify GitHub release was created
   - Count artifacts (expect 7)

2. **Check for issues**:
   - Review GitHub Actions workflow runs
   - Check for any failed builds
   - Look for reported issues since release

3. **Create follow-up issues**:
   - For any problems found during release
   - For deferred improvements identified
   - For documentation gaps
   - Use `bd create --title="..." --type=bug --priority=2` for issues

4. **Generate retrospective summary**:
   ```
   ## Release Retrospective: v<version>

   ### What went well
   - ...

   ### Issues encountered
   - ... (Issue created: beads-xxx)

   ### Action items for next release
   - ...
   ```

### 4. What's New Mode
**Invocation**: `/release-manager whats-new [version]`

Generate user-friendly release summaries:

1. **Extract from CHANGELOG**:
   - Read the version section from `CHANGELOG.md`
   - Group by category (Added, Changed, Fixed, etc.)

2. **Generate summary formats**:

   **GitHub Release Notes** (detailed):
   ```markdown
   ## Highlights
   - Key feature 1
   - Key feature 2

   ## What's Changed
   ### New Features
   - ...
   ### Bug Fixes
   - ...

   ## Installation
   cargo binstall morphir
   # or
   cargo install morphir
   ```

   **Short announcement** (social media/newsletter):
   ```
   morphir-rust v<version> is out!
   - Highlight 1
   - Highlight 2
   Download: https://github.com/finos/morphir-rust/releases/tag/v<version>
   ```

3. **Include upgrade notes** if breaking changes exist

## Mise Tasks Reference

| Task | Command | Description |
|------|---------|-------------|
| Pre-release checks | `mise run release:check` | Run tests, lint, build |
| Version bump | `mise run release:version-bump <v>` | Update Cargo.toml version |
| Validate changelog | `mise run release:changelog-validate` | Check CHANGELOG.md format |
| Add changelog entry | `mise run release:changelog-entry <cat> <msg>` | Add to Unreleased |
| Prepare changelog | `mise run release:changelog-release <v>` | Guidance for release |
| Create tag | `mise run release:tag-create <v> [--push]` | Create release tag |
| Pre-release workflow | `mise run release:pre-release <v>` | Full pre-release checklist |
| Post-release validation | `mise run release:post-release <v>` | Validate release artifacts |
| Generate all docs | `mise run docs:generate` | CLI + release notes |
| Generate release notes | `mise run docs:releases` | From CHANGELOG.md |
| Serve docs locally | `mise run docs:serve` | Jekyll dev server |

## Important Guidelines

1. **FINOS CLA**: Never add AI as commit author or co-author
2. **Changelog format**: Always use keepachangelog format (see `references/changelog-format.md`)
3. **Semantic versioning**: Follow semver strictly (see `references/version-policy.md`)
4. **Test first**: Always verify tests pass before release
5. **Draft releases**: GitHub releases are created as drafts for review

## Reference Documentation

- `references/changelog-format.md` - keepachangelog format details and examples
- `references/release-workflow.md` - Complete step-by-step release process
- `references/version-policy.md` - When to bump major/minor/patch
- `references/troubleshooting.md` - Common release issues and solutions

## Quick Start Examples

**Prepare a release**:
```
/release-manager prepare 0.2.0
```

**Analyze commits for changelog**:
```
/release-manager analyze
```

**Post-release check**:
```
/release-manager retrospective 0.1.0
```

**Generate release notes**:
```
/release-manager whats-new 0.1.0
```
