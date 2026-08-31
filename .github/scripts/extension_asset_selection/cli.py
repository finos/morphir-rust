"""Command-line dispatch for extension asset selection."""

from __future__ import annotations

import argparse
from pathlib import Path
import sys

from .bundles import select_assets, validate_matrix_bundle
from .model import AssetError
from .publication import prepare_publication

def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--bundles", type=Path, required=True)
    parser.add_argument("--existing-assets", type=Path)
    parser.add_argument("--output-manifest", type=Path)
    parser.add_argument("--expected-manifest", type=Path)
    parser.add_argument("--prepared-assets", type=Path)
    parser.add_argument("--require-all-existing", action="store_true")
    parser.add_argument("--validate-short-id")
    args = parser.parse_args()
    if args.validate_short_id is not None:
        if any(
            value is not None
            for value in (
                args.existing_assets,
                args.output_manifest,
                args.expected_manifest,
                args.prepared_assets,
            )
        ) or args.require_all_existing:
            parser.error(
                "--validate-short-id cannot be combined with publication inputs"
            )
    elif args.require_all_existing:
        if args.existing_assets is None:
            parser.error("--require-all-existing requires --existing-assets")
        if any(
            value is not None
            for value in (
                args.output_manifest,
                args.expected_manifest,
                args.prepared_assets,
            )
        ):
            parser.error("final verification cannot create publication outputs")
    elif any(
        value is None
        for value in (
            args.existing_assets,
            args.output_manifest,
            args.expected_manifest,
            args.prepared_assets,
        )
    ):
        parser.error("publication selection requires all output manifests")
    return args


def main() -> int:
    """Validate bundles and write only paths that are safe to upload."""
    args = parse_args()
    try:
        if args.validate_short_id is not None:
            validate_matrix_bundle(
                args.tag,
                args.commit,
                args.validate_short_id,
                args.root,
                args.bundles,
            )
            return 0
        selection = select_assets(
            args.tag,
            args.commit,
            args.root,
            args.bundles,
            args.existing_assets,
            require_all_existing=args.require_all_existing,
        )
        if not args.require_all_existing:
            prepare_publication(
                args.prepared_assets,
                args.output_manifest,
                args.expected_manifest,
                selection,
            )
    except AssetError as error:
        print(f"extension asset error: {error}", file=sys.stderr)
        return 2
    return 0
