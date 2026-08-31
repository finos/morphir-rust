"""Focused extension release asset selection capabilities."""

from .bundles import (
    descriptor_paths, existing_asset_map, expected_descriptor, read_descriptor,
    regular_files, select_assets, validate_bundle, validate_matrix_bundle,
)
from .cli import main, parse_args
from .model import AssetError, AssetSelection, ReleaseAsset
from .publication import (
    absolute_path, create_directory_chain, materialize_assets,
    prepare_publication, reject_symlink_components, require_absent_output,
    write_manifest_atomic,
)

__all__ = [
    "AssetError", "AssetSelection", "ReleaseAsset", "absolute_path",
    "create_directory_chain", "descriptor_paths", "existing_asset_map",
    "expected_descriptor", "main", "materialize_assets", "parse_args",
    "prepare_publication", "read_descriptor", "regular_files",
    "reject_symlink_components", "require_absent_output", "select_assets",
    "validate_bundle", "validate_matrix_bundle", "write_manifest_atomic",
]
