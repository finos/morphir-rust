# Changelog Format Reference

Based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)

## Guiding Principles

- Changelogs are for **humans**, not machines
- There should be an entry for **every version**
- Group changes by their **impact** on users
- Versions should be **linkable**
- The **latest version** comes first
- Release **date** for each version is mandatory (ISO 8601: YYYY-MM-DD)

## File Structure

```markdown
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
### Changed
### Deprecated
### Removed
### Fixed
### Security

## [1.2.3] - 2025-01-15

### Added
- New feature description ([#123](link-to-pr))

### Fixed
- Bug fix description ([#124](link-to-pr))

[Unreleased]: https://github.com/finos/morphir-rust/compare/v1.2.3...HEAD
[1.2.3]: https://github.com/finos/morphir-rust/compare/v1.2.2...v1.2.3
[1.2.2]: https://github.com/finos/morphir-rust/releases/tag/v1.2.2
```

## Change Categories

Always use these categories in this order:

| Category | Description | Example |
|----------|-------------|---------|
| **Added** | New features | New CLI command, new API |
| **Changed** | Changes to existing functionality | Updated behavior, improved performance |
| **Deprecated** | Features to be removed in future | Marked for removal |
| **Removed** | Features removed in this release | Deleted functionality |
| **Fixed** | Bug fixes | Corrected behavior |
| **Security** | Security vulnerability fixes | CVE fixes |

## Entry Format

Each entry should:
- Start with a hyphen and space (`- `)
- Be a complete sentence (can omit period)
- Reference the PR or issue when applicable
- Focus on **what changed** from the user's perspective

**Good entries**:
```markdown
- Add `morphir validate --strict` flag for stricter validation ([#45](link))
- Fix panic when parsing empty IR files ([#46](link))
- Improve build performance by 30% through parallel compilation
```

**Bad entries**:
```markdown
- Updated code  (too vague)
- Fixed bug  (which bug?)
- Refactored internals  (users don't care about internal refactors unless it affects them)
```

## Comparison Links

At the bottom of the file, add links for each version:

```markdown
[Unreleased]: https://github.com/finos/morphir-rust/compare/v1.2.3...HEAD
[1.2.3]: https://github.com/finos/morphir-rust/compare/v1.2.2...v1.2.3
[1.2.2]: https://github.com/finos/morphir-rust/compare/v1.2.1...v1.2.2
[1.2.1]: https://github.com/finos/morphir-rust/releases/tag/v1.2.1
```

The oldest version links to its release tag; all others link to comparisons.

## Workflow

1. **During development**: Add entries to `[Unreleased]` as changes are made
2. **At release time**: Move `[Unreleased]` entries to new version section
3. **After release**: Clear `[Unreleased]` categories (keep headers)

Use `mise run release:changelog-entry <category> "<entry>"` to add entries.
