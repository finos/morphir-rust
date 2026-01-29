#!/usr/bin/env python3
"""
Generate llms.txt and llms-full.txt from Morphir Rust documentation.

llms.txt format specification: https://llmstxt.org/

The llms.txt file provides a machine-readable index of documentation
for LLMs and AI agents. Two variants are generated:

- llms.txt: Compact index with links and brief descriptions
- llms-full.txt: Complete content inlined for direct consumption
"""

import argparse
import re
import sys
from pathlib import Path
from typing import NamedTuple


class DocPage(NamedTuple):
    """Represents a documentation page."""
    title: str
    path: Path
    description: str
    nav_order: int
    parent: str | None
    content: str


def extract_frontmatter(content: str) -> dict:
    """Extract YAML frontmatter from markdown content."""
    if not content.startswith('---'):
        return {}

    parts = content.split('---', 2)
    if len(parts) < 3:
        return {}

    frontmatter = {}
    for line in parts[1].strip().split('\n'):
        if ':' in line:
            key, value = line.split(':', 1)
            frontmatter[key.strip()] = value.strip().strip('"\'')

    return frontmatter


def extract_description(content: str) -> str:
    """Extract first paragraph as description."""
    # Remove frontmatter
    if content.startswith('---'):
        parts = content.split('---', 2)
        if len(parts) >= 3:
            content = parts[2]

    # Remove HTML comments
    content = re.sub(r'<!--.*?-->', '', content, flags=re.DOTALL)

    # Find first non-empty paragraph after headings
    lines = content.strip().split('\n')
    paragraph_lines = []
    in_code_block = False

    for line in lines:
        if line.strip().startswith('```'):
            in_code_block = not in_code_block
            continue
        if in_code_block:
            continue
        if line.strip().startswith('#'):
            continue
        if line.strip().startswith('{:'):  # Jekyll attributes
            continue
        if line.strip().startswith('[') and ']: ' in line:  # Link definitions
            continue
        if line.strip():
            paragraph_lines.append(line.strip())
        elif paragraph_lines:
            break

    description = ' '.join(paragraph_lines)
    # Truncate to reasonable length
    if len(description) > 200:
        description = description[:197] + '...'

    return description


def get_content_without_frontmatter(content: str) -> str:
    """Get markdown content without frontmatter."""
    if content.startswith('---'):
        parts = content.split('---', 2)
        if len(parts) >= 3:
            return parts[2].strip()
    return content.strip()


def parse_doc_file(path: Path, docs_dir: Path) -> DocPage | None:
    """Parse a markdown documentation file."""
    try:
        content = path.read_text(encoding='utf-8')
    except Exception as e:
        print(f"Warning: Could not read {path}: {e}", file=sys.stderr)
        return None

    frontmatter = extract_frontmatter(content)

    # Skip files without frontmatter (not part of navigation)
    if not frontmatter:
        return None

    # Skip excluded pages
    if frontmatter.get('nav_exclude') == 'true':
        return None

    title = frontmatter.get('title', path.stem.replace('-', ' ').title())
    nav_order = int(frontmatter.get('nav_order', 999))
    parent = frontmatter.get('parent')

    description = extract_description(content)
    clean_content = get_content_without_frontmatter(content)

    return DocPage(
        title=title,
        path=path.relative_to(docs_dir),
        description=description,
        nav_order=nav_order,
        parent=parent,
        content=clean_content,
    )


def collect_docs(docs_dir: Path) -> list[DocPage]:
    """Collect all documentation pages."""
    docs = []

    for md_file in docs_dir.rglob('*.md'):
        # Skip certain directories
        if any(part.startswith('_') for part in md_file.parts):
            continue
        if 'man' in md_file.parts:
            continue

        doc = parse_doc_file(md_file, docs_dir)
        if doc:
            docs.append(doc)

    return docs


def organize_docs(docs: list[DocPage]) -> dict[str, list[DocPage]]:
    """Organize docs into sections based on parent hierarchy."""
    sections: dict[str, list[DocPage]] = {
        'Getting Started': [],
        'CLI Reference': [],
        'Tutorials': [],
        'For Contributors': [],
        'Other': [],
    }

    for doc in docs:
        if doc.parent:
            if doc.parent in sections:
                sections[doc.parent].append(doc)
            elif any(doc.parent == d.title for d in docs):
                # Find the grandparent
                parent_doc = next((d for d in docs if d.title == doc.parent), None)
                if parent_doc and parent_doc.parent and parent_doc.parent in sections:
                    sections[parent_doc.parent].append(doc)
                else:
                    sections['Other'].append(doc)
            else:
                sections['Other'].append(doc)
        elif doc.title in sections:
            # This is a section index page, add to that section
            sections[doc.title].insert(0, doc)
        elif doc.title == 'Home':
            continue  # Skip home page, will be in header
        else:
            sections['Other'].append(doc)

    # Sort each section by nav_order
    for section in sections.values():
        section.sort(key=lambda d: (d.nav_order, d.title))

    return sections


def generate_llms_txt(docs: list[DocPage], base_url: str) -> str:
    """Generate compact llms.txt content."""
    sections = organize_docs(docs)

    lines = [
        "# Morphir Rust",
        "",
        "> Rust-based CLI tools and libraries for the Morphir ecosystem. "
        "Morphir enables functional domain modeling where you write business logic once "
        "and consume it through visualizations, code generation, type checking, and execution.",
        "",
        "Morphir Rust provides tools for working with Morphir IR (Intermediate Representation), "
        "including format migration, validation, and code generation. It supports language bindings "
        "for Gleam and includes an extension system for adding new languages and targets.",
        "",
    ]

    # Add sections
    section_order = ['Getting Started', 'CLI Reference', 'Tutorials', 'For Contributors']

    for section_name in section_order:
        section_docs = sections.get(section_name, [])
        if not section_docs:
            continue

        lines.append(f"## {section_name}")
        lines.append("")

        for doc in section_docs:
            url = f"{base_url}/{doc.path}".replace('.md', '').replace('/index', '/')
            if doc.description:
                lines.append(f"- [{doc.title}]({url}): {doc.description}")
            else:
                lines.append(f"- [{doc.title}]({url})")

        lines.append("")

    # Add optional section for less critical docs
    other_docs = sections.get('Other', [])
    if other_docs:
        lines.append("## Optional")
        lines.append("")
        for doc in other_docs:
            url = f"{base_url}/{doc.path}".replace('.md', '').replace('/index', '/')
            if doc.description:
                lines.append(f"- [{doc.title}]({url}): {doc.description}")
            else:
                lines.append(f"- [{doc.title}]({url})")
        lines.append("")

    return '\n'.join(lines)


def generate_llms_full_txt(docs: list[DocPage], base_url: str) -> str:
    """Generate full llms-full.txt with complete content."""
    sections = organize_docs(docs)

    lines = [
        "# Morphir Rust",
        "",
        "> Rust-based CLI tools and libraries for the Morphir ecosystem. "
        "Morphir enables functional domain modeling where you write business logic once "
        "and consume it through visualizations, code generation, type checking, and execution.",
        "",
        "Morphir Rust provides tools for working with Morphir IR (Intermediate Representation), "
        "including format migration, validation, and code generation. It supports language bindings "
        "for Gleam and includes an extension system for adding new languages and targets.",
        "",
        "---",
        "",
    ]

    # Add all content organized by section
    section_order = ['Getting Started', 'CLI Reference', 'Tutorials', 'For Contributors', 'Other']

    for section_name in section_order:
        section_docs = sections.get(section_name, [])
        if not section_docs:
            continue

        lines.append(f"# {section_name}")
        lines.append("")

        for doc in section_docs:
            url = f"{base_url}/{doc.path}".replace('.md', '').replace('/index', '/')
            lines.append(f"## {doc.title}")
            lines.append(f"Source: {url}")
            lines.append("")
            lines.append(doc.content)
            lines.append("")
            lines.append("---")
            lines.append("")

    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser(
        description='Generate llms.txt and llms-full.txt from documentation'
    )
    parser.add_argument(
        '--docs-dir',
        type=Path,
        default=Path('docs'),
        help='Documentation directory (default: docs)',
    )
    parser.add_argument(
        '--output-dir',
        type=Path,
        default=Path('docs'),
        help='Output directory for llms.txt files (default: docs)',
    )
    parser.add_argument(
        '--base-url',
        default='https://finos.github.io/morphir-rust',
        help='Base URL for documentation links',
    )
    parser.add_argument(
        '--compact-only',
        action='store_true',
        help='Generate only llms.txt (compact version)',
    )
    parser.add_argument(
        '--full-only',
        action='store_true',
        help='Generate only llms-full.txt',
    )

    args = parser.parse_args()

    if not args.docs_dir.exists():
        print(f"Error: Documentation directory not found: {args.docs_dir}", file=sys.stderr)
        sys.exit(1)

    args.output_dir.mkdir(parents=True, exist_ok=True)

    # Collect documentation
    print(f"Scanning {args.docs_dir} for documentation...")
    docs = collect_docs(args.docs_dir)
    print(f"Found {len(docs)} documentation pages")

    # Generate llms.txt (compact)
    if not args.full_only:
        llms_txt = generate_llms_txt(docs, args.base_url)
        output_path = args.output_dir / 'llms.txt'
        output_path.write_text(llms_txt, encoding='utf-8')
        print(f"Generated {output_path} ({len(llms_txt)} bytes)")

    # Generate llms-full.txt
    if not args.compact_only:
        llms_full_txt = generate_llms_full_txt(docs, args.base_url)
        output_path = args.output_dir / 'llms-full.txt'
        output_path.write_text(llms_full_txt, encoding='utf-8')
        print(f"Generated {output_path} ({len(llms_full_txt)} bytes)")


if __name__ == '__main__':
    main()
