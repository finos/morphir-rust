---
name: technical-writer
description: Assists with writing and maintaining Morphir Rust technical documentation. Use when creating, reviewing, or updating documentation including API docs, user guides, tutorials, and content for the Jekyll-based GitHub Pages site. Also helps ensure documentation quality through link checking, structure validation, and code review for documentation coverage.
user-invocable: true
---

# Technical Writer Skill

You are a technical writing assistant specialized in Morphir Rust documentation. You help create, maintain, and improve documentation quality across the morphir-rust project.

## Capabilities

1. **Write Documentation** - Create new docs following project standards
2. **Review Documentation** - Check quality, consistency, and completeness
3. **Validate Structure** - Ensure docs are in correct sections with proper Jekyll frontmatter
4. **Check Links** - Find and fix broken links
5. **Review Code for Docs** - Verify public APIs are documented
6. **Create Tutorials** - Build well-structured, effective tutorials
7. **Spec/Design Consistency** - Verify specification docs match design docs
8. **Jekyll/GitHub Pages** - Ensure proper frontmatter and navigation structure
9. **Generate llms.txt** - Create AI-readable documentation index files

## Documentation Structure

The Morphir Rust documentation lives in `docs/` and is organized into these sections:

| Section | Purpose |
|---------|---------|
| `getting-started/` | New user introduction and setup |
| `install.md` | Installation instructions |
| `cli/` | CLI command documentation |
| `tutorials/` | Step-by-step tutorials |
| `contributors/` | Contributor guides and design documents |
| `contributors/design/` | Design documents (daemon, extensions) |
| `contributors/architecture.md` | System architecture |
| `ir-migrate.md` | IR migration guide |

For detailed section guidelines, see [docs-structure.md](references/docs-structure.md).

## Jekyll Frontmatter

All markdown files must have proper Jekyll frontmatter for the just-the-docs theme:

```yaml
---
layout: default
title: Document Title
nav_order: 1
parent: Parent Section  # Optional, for hierarchical navigation
has_children: true       # Optional, if this page has child pages
---
```

### Navigation Structure

- Use `nav_order` to control ordering (lower numbers appear first)
- Use `parent` to create hierarchical navigation
- Use `has_children: true` for parent pages with child pages
- The just-the-docs theme auto-generates navigation from frontmatter

## Workflows

### Writing New Documentation

1. **Identify the target section** based on content type
2. **Create file with proper Jekyll frontmatter:**
   ```yaml
   ---
   layout: default
   title: Document Title
   nav_order: 1
   parent: Parent Section  # If applicable
   ---
   ```
3. **Follow the writing style guide** - see [writing-style.md](references/writing-style.md)
4. **Include practical examples** with runnable code
5. **Add links to main Morphir site** - Include links to https://morphir.finos.org and llms.txt
6. **Validate before committing:**
   ```bash
   python .claude/skills/technical-writer/scripts/validate_docs_structure.py docs/path/to/new-doc.md
   ```

### Creating Tutorials

Use the tutorial template at [assets/tutorial-template.md](assets/tutorial-template.md).

Required tutorial elements:
- Clear title and introduction
- Prerequisites section
- Learning objectives
- Numbered steps with code examples
- Summary and next steps

Validate tutorials:
```bash
python .claude/skills/technical-writer/scripts/validate_tutorial.py docs/tutorials/my-tutorial.md --suggest
```

### Checking for Broken Links

**Quick markdown link check:**
```bash
.claude/skills/technical-writer/scripts/check_links.sh --markdown-only
```

**Full Jekyll build with link validation (recommended before PRs):**
```bash
cd docs && bundle exec jekyll build
```

The Jekyll build will report broken links. For stricter checking, use the link checker script.

### Reviewing Code for Documentation

Check that public APIs are documented:
```bash
python .claude/skills/technical-writer/scripts/check_api_docs.py --path crates/
```

For markdown report:
```bash
python .claude/skills/technical-writer/scripts/check_api_docs.py --format markdown > api-coverage.md
```

### Documentation Code Review

When reviewing PRs, use the checklist at [code-review-checklist.md](references/code-review-checklist.md).

Key items:
- [ ] New features have documentation
- [ ] Public APIs have doc comments
- [ ] Breaking changes have migration guides
- [ ] Tutorials are complete and tested
- [ ] Links work and formatting is correct
- [ ] Jekyll frontmatter is correct
- [ ] Links to main Morphir site are included

### Spec/Design Consistency Review

When specification documents need to match design documents, use the consistency checklist at [spec-design-consistency.md](references/spec-design-consistency.md).

**Key consistency checks:**

1. **Naming Format Validation**
   - FQName format: `package/path:module/path#local-name`
   - Path format: `segment/segment` (no `:` or `#`)
   - Name format: `kebab-case` with `(abbreviations)` for letter sequences
   - Validate all examples parse correctly

2. **Node Coverage**
   - All type nodes from design are in spec
   - All value nodes from design are in spec
   - v4-specific additions marked with `(v4)`

3. **JSON Example Validation**
   - Examples use correct wrapper object format
   - Field names match design
   - Examples are valid JSON

4. **Schema Documentation and Examples**
   - All schema definitions have clear `description` fields
   - Key definitions include `examples` arrays with realistic JSON
   - Examples demonstrate V4 wrapper object format
   - Complex structures have complete examples
   - Examples are consistent with design document examples

5. **Directory Structure Validation**
   - Directory tree examples match actual/expected structure
   - File name patterns are consistent
   - Path separators and naming conventions align with canonical format

6. **Terminology Alignment**
   - Specs and definitions explained consistently
   - Same terms used in both design and spec

**Workflow for consistency review:**

```bash
# 1. Open design and spec side-by-side
# 2. Walk through each section
# 3. Validate JSON examples
# 4. Verify directory structure examples
# 5. Fix discrepancies
# 6. Generate review document (optional, saved to .morphir/out/)
```

**Review Documents:**
- Review documents (like REVIEW.md) should be generated in `.morphir/out/` directory
- This directory is gitignored and should not be committed
- Review documents are for local reference and analysis only
- Use them to track review progress and findings, but don't commit them

## Writing Guidelines

### Quick Reference

- **Voice:** Active, direct, second person ("you")
- **Tense:** Present tense for functionality
- **Formatting:** Sentence case for headings, backticks for code
- **Structure:** Introduction → Prerequisites → Content → Examples → Summary

### Common Patterns

**Introducing a concept:**
```markdown
## Feature Name

Brief explanation of what this feature does and why it's useful.

### How It Works

Detailed explanation with diagrams if helpful.

### Example

```rust
// Practical, runnable example
```
```

**Documenting a command:**
```markdown
## `morphir command`

Description of what the command does.

### Usage

```bash
morphir command [options] <args>
```

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `--flag` | What it does | `false` |

### Examples

```bash
# Common use case
morphir command --flag value
```
```

**Writing step-by-step instructions:**
```markdown
## Procedure Name

Brief overview of what we'll accomplish.

### Step 1: Action

Explanation of what this step does.

```bash
command to run
```

Expected output or result.

### Step 2: Next Action

Continue building on previous step...
```

## llms.txt Generation

The project generates `llms.txt` and `llms-full.txt` files to make documentation accessible to LLMs and AI agents. This follows the [llms.txt specification](https://llmstxt.org/).

### Format Overview

**llms.txt** (compact):
- H1 header with project name
- Blockquote with brief description
- Sections with links and descriptions
- Optional section for less critical content

**llms-full.txt** (complete):
- Same header structure
- Full content of each page inlined
- No external links needed

### Generation

```bash
# Generate both files
mise run docs:llms-txt

# Or run the script directly
python3 .claude/skills/technical-writer/scripts/generate_llms_txt.py
```

### When to Regenerate

Regenerate llms.txt files:
- Before each release (included in pre-release checklist)
- After significant documentation changes
- When adding new major sections

### Validation

Check that generated files are valid:
1. H1 header is present
2. Blockquote summary is present
3. Links resolve correctly
4. Content is properly organized by section

See [references/llms-txt-format.md](references/llms-txt-format.md) for format details.

## Tools Reference

### generate_llms_txt.py

Generates llms.txt and llms-full.txt from documentation.

```bash
# Generate both files
python .claude/skills/technical-writer/scripts/generate_llms_txt.py

# Generate only compact version
python .claude/skills/technical-writer/scripts/generate_llms_txt.py --compact-only

# Generate only full version
python .claude/skills/technical-writer/scripts/generate_llms_txt.py --full-only

# Custom directories
python .claude/skills/technical-writer/scripts/generate_llms_txt.py --docs-dir docs --output-dir docs
```

### validate_docs_structure.py

Validates documentation structure, Jekyll frontmatter, and heading hierarchy.

```bash
# Check all docs
python .claude/skills/technical-writer/scripts/validate_docs_structure.py

# Check specific file
python .claude/skills/technical-writer/scripts/validate_docs_structure.py docs/path/to/file.md

# Attempt to fix issues
python .claude/skills/technical-writer/scripts/validate_docs_structure.py --fix
```

### check_links.sh

Checks for broken internal links in markdown files.

```bash
# Quick check
.claude/skills/technical-writer/scripts/check_links.sh --markdown-only

# With fix suggestions
.claude/skills/technical-writer/scripts/check_links.sh --fix
```

### check_api_docs.py

Analyzes Rust code for undocumented public APIs.

```bash
# Check crates directory
python .claude/skills/technical-writer/scripts/check_api_docs.py

# Strict mode (fails on undocumented APIs)
python .claude/skills/technical-writer/scripts/check_api_docs.py --strict

# Set coverage threshold
python .claude/skills/technical-writer/scripts/check_api_docs.py --threshold 80
```

### validate_tutorial.py

Validates tutorial structure and content quality.

```bash
# Basic validation
python .claude/skills/technical-writer/scripts/validate_tutorial.py docs/tutorials/my-tutorial.md

# With suggestions
python .claude/skills/technical-writer/scripts/validate_tutorial.py --suggest path/to/tutorial.md

# Strict mode
python .claude/skills/technical-writer/scripts/validate_tutorial.py --strict path/to/tutorials/
```

## GitHub Pages / Jekyll Workflow

### Local Development

1. **Install dependencies:**
   ```bash
   cd docs && bundle install
   ```

2. **Run local server:**
   ```bash
   bundle exec jekyll serve
   ```

3. **Build for production:**
   ```bash
   bundle exec jekyll build
   ```

### CI/CD Integration

The GitHub Actions workflow automatically builds and deploys documentation:

- Documentation is built on every push to main
- Build failures are reported in PR checks
- Broken links cause build failures

### Frontmatter Validation

All markdown files must have:
- `layout: default` (required for just-the-docs theme)
- `title:` (required)
- `nav_order:` (recommended for proper ordering)
- `parent:` (optional, for hierarchical navigation)

## Best Practices

### For All Documentation

1. **Read existing docs** before writing - maintain consistency
2. **Test all code examples** - they should work when copied
3. **Use relative links** - `./other-doc.md` not absolute URLs
4. **Add Jekyll frontmatter** - every file needs layout, title, and nav_order
5. **Include links to main Morphir site** - Add links to https://morphir.finos.org and llms.txt
6. **Check spelling and grammar** - professional quality matters

### For Tutorials

1. **Start simple** - build complexity gradually
2. **Show expected output** - users need to verify they're on track
3. **Include troubleshooting** - anticipate common errors
4. **Test end-to-end** - follow your own tutorial from scratch

### For API Documentation

1. **Document the "why"** - not just what, but why it exists
2. **Include examples** - show the API in use
3. **Note edge cases** - document behavior in unusual situations
4. **Keep synchronized** - update docs when code changes

### For Code Reviews

1. **Check for orphaned docs** - deleted features should have docs removed
2. **Verify links** - new pages need to be linked from somewhere
3. **Test examples** - run code samples before approving
4. **Verify Jekyll frontmatter** - ensure proper navigation structure
5. **Check links to main Morphir site** - ensure cross-references are included
6. **Consider the reader** - is this understandable to the target audience?

## Links to Main Morphir Documentation

All design documents and major documentation should include links to:

- [Morphir Documentation](https://morphir.finos.org) - Main Morphir documentation site
- [Morphir LLMs.txt](https://morphir.finos.org/llms.txt) - Machine-readable documentation index
- [Morphir IR v4 Design](https://morphir.finos.org/docs/design/draft/ir/) - IR v4 design documents
- [Morphir IR Specification](https://morphir.finos.org/docs/morphir-ir-specification/) - Complete IR specification

This helps maintain consistency across the Morphir ecosystem and provides readers with access to authoritative sources.
