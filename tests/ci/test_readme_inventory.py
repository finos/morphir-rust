"""Verify source-checked README identities, membership, and versions.

Inventory descriptions remain curated documentation.
"""

from __future__ import annotations

from collections import Counter
import json
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
README_PATH = REPOSITORY_ROOT / "README.md"
EXTENSION_REGISTRY_PATH = REPOSITORY_ROOT / ".github" / "extensions.toml"
LEVEL_TWO_HEADING = re.compile(r"^##(?!#)(?:\s+.*)?$")
TABLE_SEPARATOR = re.compile(r"^:?-{3,}:?$")


def _section_lines(markdown: str, heading: str) -> list[str]:
    lines = markdown.splitlines()
    matches = [index for index, line in enumerate(lines) if line == heading]
    if len(matches) != 1:
        raise AssertionError(
            f"expected exactly one {heading!r} section; found {len(matches)}"
        )

    start = matches[0] + 1
    end = next(
        (
            index
            for index in range(start, len(lines))
            if LEVEL_TWO_HEADING.fullmatch(lines[index])
        ),
        len(lines),
    )
    return lines[start:end]


def _table_cells(line: str) -> tuple[str, ...]:
    stripped = line.strip()
    if not stripped.startswith("|") or not stripped.endswith("|"):
        raise AssertionError(f"malformed Markdown table row: {line!r}")

    cells: list[str] = []
    current: list[str] = []
    content = stripped[1:-1]
    index = 0
    while index < len(content):
        if content[index : index + 2] == r"\|":
            current.append("|")
            index += 2
        elif content[index] == "|":
            cells.append("".join(current).strip())
            current = []
            index += 1
        else:
            current.append(content[index])
            index += 1
    cells.append("".join(current).strip())
    return tuple(cells)


def _strip_code_backticks(value: str) -> str:
    if len(value) >= 2 and value.startswith("`") and value.endswith("`"):
        return value[1:-1].strip()
    return value


def _parse_table(
    markdown: str,
    heading: str,
    headers: tuple[str, ...],
    code_columns: frozenset[int],
) -> list[tuple[str, ...]]:
    section = _section_lines(markdown, heading)
    table_starts = [
        index for index, line in enumerate(section) if line.strip().startswith("|")
    ]
    if not table_starts:
        raise AssertionError(f"{heading!r} has no Markdown table")

    header_index = table_starts[0]
    actual_headers = _table_cells(section[header_index])
    if actual_headers != headers:
        raise AssertionError(
            f"{heading!r} table headers differ: expected {headers!r}, "
            f"found {actual_headers!r}"
        )
    if header_index + 1 >= len(section):
        raise AssertionError(f"{heading!r} table is missing its separator row")

    separators = _table_cells(section[header_index + 1])
    if len(separators) != len(headers) or any(
        not TABLE_SEPARATOR.fullmatch(cell) for cell in separators
    ):
        raise AssertionError(
            f"{heading!r} has a malformed table separator: {separators!r}"
        )

    rows: list[tuple[str, ...]] = []
    row_index = header_index + 2
    while row_index < len(section) and section[row_index].strip().startswith("|"):
        cells = _table_cells(section[row_index])
        if len(cells) != len(headers):
            raise AssertionError(
                f"{heading!r} has a malformed row with {len(cells)} cells: "
                f"{section[row_index]!r}"
            )
        normalized = tuple(
            _strip_code_backticks(cell) if index in code_columns else cell
            for index, cell in enumerate(cells)
        )
        if any(not cell for cell in normalized):
            raise AssertionError(f"{heading!r} has an empty table cell: {cells!r}")
        rows.append(normalized)
        row_index += 1

    if not rows:
        raise AssertionError(f"{heading!r} table is empty")

    identifiers = [row[0] for row in rows]
    duplicates = sorted(
        identifier
        for identifier, count in Counter(identifiers).items()
        if count > 1
    )
    if duplicates:
        raise AssertionError(f"{heading!r} has duplicate rows: {duplicates!r}")
    return rows


def _load_toml(path: Path) -> dict:
    with path.open("rb") as source:
        return tomllib.load(source)


def _cargo_metadata(repository_root: Path) -> dict:
    arguments = [
        "cargo",
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--locked",
    ]
    try:
        result = subprocess.run(
            arguments,
            cwd=repository_root,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise AssertionError(
            f"could not run {' '.join(arguments)!r} in {repository_root}: {error}"
        ) from error
    if result.returncode != 0:
        raise AssertionError(
            f"{' '.join(arguments)!r} failed in {repository_root} with exit code "
            f"{result.returncode}; stdout={result.stdout!r}; stderr={result.stderr!r}"
        )
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise AssertionError(
            f"cargo metadata returned invalid JSON in {repository_root}: {error}; "
            f"stdout={result.stdout!r}; stderr={result.stderr!r}"
        ) from error
    if not isinstance(metadata, dict):
        raise AssertionError(
            f"cargo metadata returned {type(metadata).__name__}, expected an object"
        )
    return metadata


def _workspace_packages(
    repository_root: Path = REPOSITORY_ROOT,
) -> tuple[dict, dict[str, dict]]:
    workspace_manifest = _load_toml(repository_root / "Cargo.toml")
    metadata = _cargo_metadata(repository_root)
    workspace_member_ids = metadata.get("workspace_members")
    metadata_packages = metadata.get("packages")
    if not isinstance(workspace_member_ids, list) or not all(
        isinstance(package_id, str) for package_id in workspace_member_ids
    ):
        raise AssertionError(
            "cargo metadata has an invalid or missing workspace_members list"
        )
    if not isinstance(metadata_packages, list) or not all(
        isinstance(package, dict) for package in metadata_packages
    ):
        raise AssertionError("cargo metadata has an invalid or missing packages list")

    packages_by_id = {
        package["id"]: package
        for package in metadata_packages
        if isinstance(package.get("id"), str)
    }
    missing_ids = sorted(set(workspace_member_ids) - set(packages_by_id))
    if missing_ids:
        raise AssertionError(
            f"cargo metadata omits workspace member package records: {missing_ids!r}"
        )

    member_packages = [
        packages_by_id[package_id] for package_id in workspace_member_ids
    ]
    package_names = [package.get("name") for package in member_packages]
    if not all(isinstance(name, str) for name in package_names):
        raise AssertionError("cargo metadata workspace package has an invalid name")
    duplicates = sorted(
        name for name, count in Counter(package_names).items() if count > 1
    )
    if duplicates:
        raise AssertionError(f"workspace has duplicate package names: {duplicates!r}")

    packages: dict[str, dict] = {}
    for metadata_package in member_packages:
        manifest_path = metadata_package.get("manifest_path")
        if not isinstance(manifest_path, str):
            raise AssertionError(
                f"cargo metadata package {metadata_package.get('id')!r} has no "
                "manifest_path"
            )
        manifest_package = _load_toml(Path(manifest_path))["package"]
        metadata_name = metadata_package["name"]
        if manifest_package.get("name") != metadata_name:
            raise AssertionError(
                f"cargo metadata names {manifest_path!r} as {metadata_name!r}, "
                f"but its manifest names {manifest_package.get('name')!r}"
            )
        packages[metadata_name] = manifest_package
    return workspace_manifest, packages


def _package_version(package: dict, workspace_manifest: dict) -> str:
    version = package.get("version")
    if isinstance(version, str):
        return version
    if version == {"workspace": True}:
        workspace_version = workspace_manifest["workspace"]["package"]["version"]
        if isinstance(workspace_version, str):
            return workspace_version
    raise AssertionError(
        f"package {package.get('name')!r} has an unsupported version declaration: "
        f"{version!r}"
    )


def _membership_message(kind: str, expected: set[str], actual: set[str]) -> str:
    return (
        f"{kind} inventory differs from source metadata; "
        f"missing={sorted(expected - actual)!r}, extra={sorted(actual - expected)!r}"
    )


class MarkdownInventoryParserTests(unittest.TestCase):
    def test_parses_escaped_pipe_in_table_cell(self) -> None:
        markdown = (
            "## Workspace crates\n\n"
            "| Crate | Description |\n"
            "| --- | --- |\n"
            "| `one` | Reads left \\| right. |\n"
        )

        self.assertEqual(
            [("one", "Reads left | right.")],
            _parse_table(
                markdown,
                "## Workspace crates",
                ("Crate", "Description"),
                frozenset({0}),
            ),
        )

    def test_stops_after_table_before_later_prose(self) -> None:
        markdown = (
            "## Workspace crates\n\n"
            "| Crate | Description |\n"
            "| --- | --- |\n"
            "| `one` | First crate. |\n\n"
            "Ordinary prose after the inventory.\n"
        )

        self.assertEqual(
            [("one", "First crate.")],
            _parse_table(
                markdown,
                "## Workspace crates",
                ("Crate", "Description"),
                frozenset({0}),
            ),
        )

    def test_rejects_missing_and_duplicate_sections(self) -> None:
        with self.assertRaisesRegex(AssertionError, "found 0"):
            _section_lines("# README\n", "## Workspace crates")
        with self.assertRaisesRegex(AssertionError, "found 2"):
            _section_lines(
                "## Workspace crates\n\n## Workspace crates\n",
                "## Workspace crates",
            )

    def test_rejects_missing_empty_or_malformed_tables(self) -> None:
        cases = {
            "missing": "Ordinary prose.\n",
            "empty": "| Crate | Description |\n| --- | --- |\n",
            "malformed header": "| Wrong | Description |\n| --- | --- |\n",
            "malformed separator": (
                "| Crate | Description |\n| -- | --- |\n| `one` | First |\n"
            ),
            "malformed data": (
                "| Crate | Description |\n| --- | --- |\n| `one` |\n"
            ),
            "unterminated data": (
                "| Crate | Description |\n| --- | --- |\n| `one` | First\n"
            ),
            "duplicate": (
                "| Crate | Description |\n| --- | --- |\n"
                "| `one` | First |\n| `one` | Second |\n"
            ),
            "empty description": (
                "| Crate | Description |\n| --- | --- |\n| `one` | |\n"
            ),
        }
        for name, table in cases.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                _parse_table(
                    f"## Workspace crates\n\n{table}",
                    "## Workspace crates",
                    ("Crate", "Description"),
                    frozenset({0}),
                )


class WorkspaceMetadataTests(unittest.TestCase):
    def test_uses_cargo_membership_for_excludes_and_in_tree_path_dependencies(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repository_root = Path(temporary)
            (repository_root / "Cargo.toml").write_text(
                """\
[workspace]
members = ["crates/*"]
exclude = ["crates/excluded"]
resolver = "2"
""",
                encoding="utf-8",
            )
            self._write_package(
                repository_root / "crates" / "included",
                "included",
                'path-helper = { path = "../../path-helper" }',
            )
            self._write_package(
                repository_root / "crates" / "excluded", "excluded"
            )
            self._write_package(repository_root / "path-helper", "path-helper")
            subprocess.run(
                ["cargo", "generate-lockfile", "--offline"],
                cwd=repository_root,
                check=True,
                capture_output=True,
                text=True,
            )

            _workspace_manifest, packages = _workspace_packages(repository_root)

            self.assertEqual({"included", "path-helper"}, set(packages))

    @staticmethod
    def _write_package(path: Path, name: str, dependency: str = "") -> None:
        (path / "src").mkdir(parents=True)
        dependencies = f"\n[dependencies]\n{dependency}\n" if dependency else ""
        (path / "Cargo.toml").write_text(
            f"""\
[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
{dependencies}""",
            encoding="utf-8",
        )
        (path / "src" / "lib.rs").write_text("", encoding="utf-8")


class ReadmeInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.readme = README_PATH.read_text(encoding="utf-8")
        cls.workspace_manifest, cls.packages = _workspace_packages()

    def test_workspace_crates_match_workspace_manifests(self) -> None:
        rows = _parse_table(
            self.readme,
            "## Workspace crates",
            ("Crate", "Description"),
            frozenset({0}),
        )
        identifiers = [row[0] for row in rows]
        self.assertEqual(
            sorted(identifiers),
            identifiers,
            "workspace crate inventory must be sorted by crate name",
        )
        actual_names = set(identifiers)
        expected_names = set(self.packages)
        self.assertEqual(
            expected_names,
            actual_names,
            _membership_message("workspace crate", expected_names, actual_names),
        )

    def test_extensions_match_registry_packages_and_versions(self) -> None:
        rows = _parse_table(
            self.readme,
            "## Extensions",
            ("Extension", "Package", "Version", "Description"),
            frozenset({0, 1, 2}),
        )
        identifiers = [row[0] for row in rows]
        self.assertEqual(
            sorted(identifiers),
            identifiers,
            "extension inventory must be sorted by extension ID",
        )
        actual = {
            extension_id: {"package": package, "version": version}
            for extension_id, package, version, _description in rows
        }

        registry = _load_toml(EXTENSION_REGISTRY_PATH).get("extensions", {})
        expected: dict[str, dict[str, str]] = {}
        for registry_name, extension in registry.items():
            extension_id = extension["extension_id"]
            if extension_id in expected:
                raise AssertionError(
                    f"extension registry has duplicate extension_id {extension_id!r}"
                )
            package_name = extension["package"]
            if package_name not in self.packages:
                raise AssertionError(
                    f"registered extension {registry_name!r} references unknown "
                    "package "
                    f"{package_name!r}"
                )
            expected[extension_id] = {
                "package": package_name,
                "version": _package_version(
                    self.packages[package_name], self.workspace_manifest
                ),
            }

        expected_ids = set(expected)
        actual_ids = set(actual)
        self.assertEqual(
            expected_ids,
            actual_ids,
            _membership_message("extension", expected_ids, actual_ids),
        )
        package_mismatches = {
            extension_id: {
                "expected": expected[extension_id]["package"],
                "actual": actual[extension_id]["package"],
            }
            for extension_id in expected_ids & actual_ids
            if expected[extension_id]["package"] != actual[extension_id]["package"]
        }
        self.assertFalse(
            package_mismatches,
            f"extension package mappings differ: {package_mismatches!r}",
        )
        version_mismatches = {
            extension_id: {
                "expected": expected[extension_id]["version"],
                "actual": actual[extension_id]["version"],
            }
            for extension_id in expected_ids & actual_ids
            if expected[extension_id]["version"] != actual[extension_id]["version"]
        }
        self.assertFalse(
            version_mismatches,
            f"extension version mappings differ: {version_mismatches!r}",
        )


if __name__ == "__main__":
    unittest.main()
