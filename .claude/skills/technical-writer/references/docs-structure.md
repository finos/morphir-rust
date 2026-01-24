# Morphir Rust Documentation Structure Guide

This guide describes the organization of Morphir Rust documentation and where different types of content should be placed.

## Documentation Sections

### getting-started.md
**Purpose:** Entry point for new users

**Content Types:**
- Introduction and overview
- Quick start guide
- First steps with Morphir Rust

**Guidelines:**
- Keep content beginner-friendly
- Avoid jargon or explain it immediately
- Include code examples
- Link to deeper documentation for advanced topics

### install.md
**Purpose:** Installation instructions

**Content Types:**
- Installation methods (cargo, pre-built binaries)
- System requirements
- Verification steps
- Troubleshooting

**Guidelines:**
- Provide clear step-by-step instructions
- Include platform-specific notes
- Test installation steps regularly

### cli/
**Purpose:** CLI command documentation

**Content Types:**
- Command reference
- Subcommand documentation
- Option descriptions
- Examples

**Guidelines:**
- Keep command documentation up-to-date with CLI changes
- Include practical examples for each command
- Document flags and options clearly
- Use consistent formatting

### tutorials/
**Purpose:** Step-by-step tutorials

**Content Types:**
- Getting started tutorials
- Feature-specific tutorials
- Complete workflow examples
- Extension development guides

**Guidelines:**
- Task-oriented content
- Step-by-step instructions
- Include working examples
- Test tutorials end-to-end

### contributors/
**Purpose:** Documentation for contributors

**Subdirectories:**
- `design/` - Design documents (daemon, extensions)
- Architecture documentation
- Development guides

**Content Types:**
- Architecture overview
- Extension system design
- CLI architecture
- Design documents
- Development setup

**Guidelines:**
- Technical depth appropriate for contributors
- Include design rationale
- Link to implementation code
- Keep up-to-date with codebase

### contributors/design/
**Purpose:** Design documents for Morphir Rust features

**Subdirectories:**
- `daemon/` - Daemon design documents
- `extensions/` - Extension system design

**Content Types:**
- Feature designs
- Architecture proposals
- Protocol specifications
- Configuration specifications

**Guidelines:**
- Include links to main Morphir documentation
- Reference Morphir IR specifications
- Include JSON examples where applicable
- Maintain consistency with main Morphir design docs

## File Naming Conventions

- Use lowercase with hyphens: `my-document.md`
- Be descriptive but concise
- Avoid special characters
- Use `index.md` for directory landing pages
- Use `README.md` for design document directories

## Jekyll Frontmatter Requirements

Every markdown file must have Jekyll frontmatter for the just-the-docs theme:

```yaml
---
layout: default
title: Document Title
nav_order: 1
parent: Parent Section  # Optional, for hierarchical navigation
has_children: true       # Optional, if this page has child pages
---
```

### Required Fields

- `layout: default` - Required for just-the-docs theme
- `title:` - Required, used for page title and navigation

### Optional Fields

- `nav_order:` - Controls ordering in navigation (lower numbers appear first)
- `parent:` - Creates hierarchical navigation structure
- `has_children: true` - Indicates this page has child pages

### Navigation Structure

The just-the-docs theme automatically generates navigation from frontmatter:
- Pages are ordered by `nav_order`
- Parent-child relationships are created with `parent`
- Child pages appear nested under their parent

## Cross-Referencing

### Internal Links

- Use relative paths for internal links
- Include file extension: `[link](./other-doc.md)`
- For directories, link to `index.md` or `README.md`
- Check links before committing

### External Links

- Include links to main Morphir documentation: https://morphir.finos.org
- Link to Morphir LLMs.txt: https://morphir.finos.org/llms.txt
- Link to Morphir IR specifications when relevant
- Use descriptive link text

### Links to Main Morphir Site

All design documents should include a "Related" section with:
- Links to main Morphir documentation
- Links to Morphir LLMs.txt
- Links to relevant IR specifications
- Links to related design documents in main repo

## Documentation Organization Principles

1. **User-Focused** - Organize by user needs, not internal structure
2. **Progressive Disclosure** - Start simple, add complexity gradually
3. **Findability** - Use clear titles and consistent structure
4. **Maintainability** - Keep related content together
5. **Cross-Reference** - Link to related content and main Morphir docs
