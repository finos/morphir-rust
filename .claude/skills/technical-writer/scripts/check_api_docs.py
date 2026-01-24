#!/usr/bin/env python3
"""
check_api_docs.py - Check that public APIs are documented

This script analyzes Rust source code to identify public APIs (pub functions,
types, methods) and verifies they have proper documentation comments.

Usage:
    python check_api_docs.py [--path PATH] [--format FORMAT] [--strict]

Options:
    --path PATH     Path to check (default: crates/)
    --format FORMAT Output format: text, json, markdown (default: text)
    --strict        Fail if any undocumented exports are found
    --threshold     Minimum documentation coverage percentage (default: 0)

Exit codes:
    0 - All public APIs are documented (or not in strict mode)
    1 - Undocumented public APIs found (in strict mode)
    2 - Script error
"""

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import List, Dict, Optional, Tuple

@dataclass
class PublicAPI:
    """Represents a public API element."""
    file_path: str
    line_number: int
    api_type: str  # function, struct, enum, trait, impl, const, static
    name: str
    signature: str
    has_doc: bool
    doc_comment: Optional[str] = None


def is_rust_public(line: str) -> bool:
    """Check if a Rust item is public (starts with 'pub')."""
    return bool(re.match(r'^\s*pub\s+', line))


def extract_doc_comment(lines: List[str], target_line: int) -> Tuple[bool, Optional[str]]:
    """Extract documentation comment preceding a declaration."""
    doc_lines = []
    i = target_line - 2  # 0-indexed, look at line before

    # Walk backwards collecting doc comment lines
    while i >= 0:
        line = lines[i].strip()
        if line.startswith('///'):
            doc_lines.insert(0, line[3:].strip())
            i -= 1
        elif line.startswith('//!'):
            # Module-level docs
            doc_lines.insert(0, line[3:].strip())
            i -= 1
        elif not line:
            # Empty line breaks the doc comment chain
            break
        else:
            break

    if doc_lines:
        return True, '\n'.join(doc_lines)
    return False, None


def parse_rust_file(file_path: Path) -> List[PublicAPI]:
    """Parse a Rust file and extract public API elements."""
    apis = []

    try:
        content = file_path.read_text(encoding='utf-8')
        lines = content.split('\n')
    except Exception:
        return apis

    # Patterns for Rust declarations
    pub_func_pattern = re.compile(r'^\s*pub\s+(?:async\s+)?fn\s+(\w+)')
    pub_struct_pattern = re.compile(r'^\s*pub\s+struct\s+(\w+)')
    pub_enum_pattern = re.compile(r'^\s*pub\s+enum\s+(\w+)')
    pub_trait_pattern = re.compile(r'^\s*pub\s+trait\s+(\w+)')
    pub_impl_pattern = re.compile(r'^\s*pub\s+impl\s+(?:.*\s+for\s+)?(\w+)')
    pub_const_pattern = re.compile(r'^\s*pub\s+const\s+(\w+)')
    pub_static_pattern = re.compile(r'^\s*pub\s+static\s+(\w+)')

    for i, line in enumerate(lines, 1):
        stripped = line.strip()

        # Check for public function
        match = pub_func_pattern.match(stripped)
        if match:
            name = match.group(1)
            has_doc, doc_comment = extract_doc_comment(lines, i)
            apis.append(PublicAPI(
                file_path=str(file_path),
                line_number=i,
                api_type='function',
                name=name,
                signature=stripped[:100],  # First 100 chars
                has_doc=has_doc,
                doc_comment=doc_comment
            ))
            continue

        # Check for public struct
        match = pub_struct_pattern.match(stripped)
        if match:
            name = match.group(1)
            has_doc, doc_comment = extract_doc_comment(lines, i)
            apis.append(PublicAPI(
                file_path=str(file_path),
                line_number=i,
                api_type='struct',
                name=name,
                signature=stripped[:100],
                has_doc=has_doc,
                doc_comment=doc_comment
            ))
            continue

        # Check for public enum
        match = pub_enum_pattern.match(stripped)
        if match:
            name = match.group(1)
            has_doc, doc_comment = extract_doc_comment(lines, i)
            apis.append(PublicAPI(
                file_path=str(file_path),
                line_number=i,
                api_type='enum',
                name=name,
                signature=stripped[:100],
                has_doc=has_doc,
                doc_comment=doc_comment
            ))
            continue

        # Check for public trait
        match = pub_trait_pattern.match(stripped)
        if match:
            name = match.group(1)
            has_doc, doc_comment = extract_doc_comment(lines, i)
            apis.append(PublicAPI(
                file_path=str(file_path),
                line_number=i,
                api_type='trait',
                name=name,
                signature=stripped[:100],
                has_doc=has_doc,
                doc_comment=doc_comment
            ))
            continue

        # Check for public const
        match = pub_const_pattern.match(stripped)
        if match:
            name = match.group(1)
            has_doc, doc_comment = extract_doc_comment(lines, i)
            apis.append(PublicAPI(
                file_path=str(file_path),
                line_number=i,
                api_type='const',
                name=name,
                signature=stripped[:100],
                has_doc=has_doc,
                doc_comment=doc_comment
            ))
            continue

        # Check for public static
        match = pub_static_pattern.match(stripped)
        if match:
            name = match.group(1)
            has_doc, doc_comment = extract_doc_comment(lines, i)
            apis.append(PublicAPI(
                file_path=str(file_path),
                line_number=i,
                api_type='static',
                name=name,
                signature=stripped[:100],
                has_doc=has_doc,
                doc_comment=doc_comment
            ))
            continue

    return apis


def scan_directory(path: Path) -> List[PublicAPI]:
    """Scan a directory for Rust files and extract public APIs."""
    all_apis = []

    for rust_file in path.rglob('*.rs'):
        # Skip test files and examples
        if 'test' in rust_file.parts or 'example' in rust_file.parts:
            continue
        apis = parse_rust_file(rust_file)
        all_apis.extend(apis)

    return all_apis


def main():
    parser = argparse.ArgumentParser(
        description="Check that public Rust APIs are documented"
    )
    parser.add_argument('--path', default='crates/',
                        help="Path to check (default: crates/)")
    parser.add_argument('--format', choices=['text', 'json', 'markdown'],
                        default='text',
                        help="Output format (default: text)")
    parser.add_argument('--strict', action='store_true',
                        help="Fail if any undocumented APIs found")
    parser.add_argument('--threshold', type=int, default=0,
                        help="Minimum documentation coverage percentage")

    args = parser.parse_args()

    # Find project root
    script_dir = Path(__file__).parent
    project_root = script_dir.parent.parent.parent.parent

    target_path = project_root / args.path
    if not target_path.exists():
        print(f"Error: Path not found: {target_path}")
        sys.exit(2)

    print(f"Scanning for public APIs in: {target_path}")
    print("")

    apis = scan_directory(target_path)

    if not apis:
        print("No public APIs found.")
        sys.exit(0)

    documented = [api for api in apis if api.has_doc]
    undocumented = [api for api in apis if not api.has_doc]

    coverage = (len(documented) / len(apis)) * 100 if apis else 0

    if args.format == 'json':
        output = {
            'total': len(apis),
            'documented': len(documented),
            'undocumented': len(undocumented),
            'coverage': coverage,
            'apis': [asdict(api) for api in apis]
        }
        print(json.dumps(output, indent=2))
    elif args.format == 'markdown':
        print("# API Documentation Coverage Report\n")
        print(f"- **Total APIs**: {len(apis)}")
        print(f"- **Documented**: {len(documented)}")
        print(f"- **Undocumented**: {len(undocumented)}")
        print(f"- **Coverage**: {coverage:.1f}%\n")
        if undocumented:
            print("## Undocumented APIs\n")
            for api in undocumented:
                print(f"- `{api.name}` ({api.api_type}) in {api.file_path}:{api.line_number}")
    else:
        print(f"Total public APIs: {len(apis)}")
        print(f"Documented: {len(documented)}")
        print(f"Undocumented: {len(undocumented)}")
        print(f"Coverage: {coverage:.1f}%")
        print("")

        if undocumented:
            print("Undocumented APIs:")
            for api in undocumented[:20]:  # Limit output
                print(f"  {api.api_type}: {api.name} in {api.file_path}:{api.line_number}")
            if len(undocumented) > 20:
                print(f"  ... and {len(undocumented) - 20} more")

    if coverage < args.threshold:
        print(f"\nCoverage {coverage:.1f}% is below threshold of {args.threshold}%")
        sys.exit(1)

    if args.strict and undocumented:
        print(f"\nFound {len(undocumented)} undocumented public APIs")
        sys.exit(1)

    sys.exit(0)


if __name__ == "__main__":
    main()
