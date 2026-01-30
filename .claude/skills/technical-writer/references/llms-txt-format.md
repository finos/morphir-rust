# llms.txt Format Reference

The llms.txt specification provides a standardized way for websites to provide key information to LLMs and AI agents.

**Specification**: https://llmstxt.org/
**Original Proposal**: https://www.answer.ai/posts/2024-09-03-llmstxt.html

## File Locations

- `/llms.txt` - Compact index with links and descriptions
- `/llms-full.txt` - Complete content inlined

## Format Structure

```markdown
# Project Name

> Brief description of the project (1-2 sentences)

Additional context paragraph explaining what the project does,
key features, and primary use cases.

## Section Name

- [Page Title](https://url/to/page): Brief description of the page content

## Another Section

- [Page Title](https://url/to/page): Description
- [Page Title](https://url/to/page): Description

## Optional

- [Less Critical Page](https://url/to/page): Description
```

## Required Elements

1. **H1 Header** - Project or site name (only required element)

## Optional Elements

1. **Blockquote** - Short summary with key information
2. **Detail Paragraphs** - Additional context (no headings)
3. **Sections** - H2-delimited groups of links
4. **Optional Section** - Links that can be skipped for shorter context

## Link Format

```markdown
- [Title](url): Optional description
```

The description after the colon is optional but recommended for context.

## Section Organization for Morphir Rust

| Section | Content |
|---------|---------|
| Getting Started | Installation, quick start, basic usage |
| CLI Reference | Command documentation |
| Tutorials | Step-by-step guides |
| For Contributors | Architecture, design docs, development guides |
| Optional | Release notes, edge cases, reference material |

## llms-full.txt Differences

The full version includes complete page content:

```markdown
# Project Name

> Description

---

# Section Name

## Page Title
Source: https://url/to/page

[Full markdown content of the page]

---

## Another Page Title
Source: https://url/to/page

[Full markdown content]
```

## Best Practices

1. **Keep descriptions concise** - 1 sentence, under 100 characters
2. **Order sections by importance** - Most useful content first
3. **Use Optional section** - For content LLMs can skip when context is limited
4. **Update regularly** - Regenerate before releases
5. **Validate links** - Ensure all URLs are accessible

## Generation

```bash
# Generate both files
mise run docs:llms-txt

# Using the script directly
python3 .claude/skills/technical-writer/scripts/generate_llms_txt.py \
    --docs-dir docs \
    --output-dir docs \
    --base-url https://finos.github.io/morphir-rust
```

## Integration with Releases

llms.txt generation is part of the pre-release checklist:

1. Documentation updates complete
2. Run `mise run docs:llms-txt`
3. Verify generated files
4. Commit with other release changes
