"""Command-line dispatch for extension bundle packaging."""

from __future__ import annotations

import argparse
from pathlib import Path
import stat
import sys

from .bundles import verified_bundle, write_bundle
from .errors import PackageError
from .model import expected_bundle, package_version, registered_extension, require_identifier
from .paths import clean_extension_staging, clean_head_snapshot, validate_extension_staging


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build a deterministic release bundle for a registered WASM extension."
    )
    parser.add_argument("--clean-extension-staging")
    parser.add_argument("--validate-extension-staging")
    parser.add_argument("--clean-head-snapshot", type=Path)
    parser.add_argument("--transfer-bundle", type=Path)
    parser.add_argument("--short-id")
    parser.add_argument("--wasm", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--git-commit")
    args = parser.parse_args()
    operation_count = sum(
        (
            args.clean_extension_staging is not None,
            args.validate_extension_staging is not None,
            args.clean_head_snapshot is not None,
            args.transfer_bundle is not None,
        )
    )
    if operation_count > 1:
        parser.error("only one cleanup or transfer mode may be selected")
    if (
        args.clean_extension_staging is not None
        or args.validate_extension_staging is not None
    ):
        if any(
            value is not None
            for value in (
                args.short_id,
                args.wasm,
                args.output,
                args.git_commit,
                args.transfer_bundle,
            )
        ):
            parser.error("cleanup modes cannot be combined with other arguments")
    elif args.clean_head_snapshot is not None:
        if args.short_id is None:
            parser.error("the following arguments are required: --short-id")
        if any(
            value is not None
            for value in (args.wasm, args.output, args.git_commit, args.transfer_bundle)
        ):
            parser.error("cleanup modes cannot be combined with other arguments")
    elif args.transfer_bundle is not None:
        if args.output is None:
            parser.error("the following arguments are required: --output")
        if any(value is not None for value in (args.short_id, args.wasm, args.git_commit)):
            parser.error("transfer mode cannot be combined with packaging arguments")
    else:
        missing = [
            option
            for option, value in (
                ("--short-id", args.short_id),
                ("--wasm", args.wasm),
                ("--output", args.output),
            )
            if value is None
        ]
        if missing:
            parser.error(f"the following arguments are required: {', '.join(missing)}")
    return args


def package(root: Path, args: argparse.Namespace) -> None:
    extension = registered_extension(root, args.short_id)
    registered_package = require_identifier(extension.get("package"), "package")

    try:
        wasm_status = args.wasm.stat()
    except FileNotFoundError as error:
        raise PackageError(f"WASM file does not exist: {args.wasm}") from error
    except OSError as error:
        raise PackageError(f"cannot inspect WASM file {args.wasm}: {error}") from error
    if not stat.S_ISREG(wasm_status.st_mode):
        raise PackageError(f"WASM file does not exist: {args.wasm}")

    version = package_version(root, registered_package)
    try:
        wasm_bytes = args.wasm.read_bytes()
    except OSError as error:
        raise PackageError(f"cannot read WASM file {args.wasm}: {error}") from error
    expected = expected_bundle(
        args.short_id,
        extension,
        version,
        wasm_bytes,
        args.git_commit,
    )
    write_bundle(args.output, expected)


def transfer_bundle(source: Path, output: Path) -> None:
    write_bundle(output, verified_bundle(source))


def main() -> int:
    try:
        args = parse_args()
        root = Path(__file__).resolve().parents[2]
        if args.clean_extension_staging is not None:
            clean_extension_staging(root, args.clean_extension_staging)
        elif args.validate_extension_staging is not None:
            validate_extension_staging(root, args.validate_extension_staging)
        elif args.clean_head_snapshot is not None:
            clean_head_snapshot(root, args.clean_head_snapshot, args.short_id)
        elif args.transfer_bundle is not None:
            transfer_bundle(args.transfer_bundle, args.output)
        else:
            package(root, args)
    except PackageError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0
