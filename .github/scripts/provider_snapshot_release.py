#!/usr/bin/env python3
"""Package, describe and verify native provider snapshot release assets.

A provider-snapshot/v<semver> tag publishes two executables per target as
separate archives: the production `morphir-provider-snapshot` service and the
test-only `provider-snapshot-fixture` exporter. One descriptor records the
source commit, target, build mode, compiler and every archive and executable
digest, with the protocol facts observed by packaged qualification.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tarfile
from typing import Any, Mapping

from extension_release import COMMIT_PATTERN, SEMVER, ReleaseError, load_toml, validate_semver

TAG_PATTERN = re.compile(rf"^provider-snapshot/v(?P<version>{SEMVER})$")
PACKAGE = "morphir-package"
DESCRIPTOR_SCHEMA = "0.1.0-draft.1"
BUILD_MODE = "release"
VARIANTS = (
    "compatible",
    "wrong-type",
    "wrong-subject-role",
    "wrong-object-form",
    "wrong-interpreter",
    "ambiguous-contexts",
)


@dataclass(frozen=True)
class Target:
    """One qualified native target and the runner image that builds it."""

    triple: str
    os: str
    arch: str
    runner: str


TARGETS = {
    target.triple: target
    for target in (
        Target("x86_64-unknown-linux-gnu", "linux", "x86_64", "ubuntu-24.04"),
        Target("aarch64-apple-darwin", "macos", "aarch64", "macos-15"),
    )
}


@dataclass(frozen=True)
class Role:
    """One executable role. Production and test tooling never share an archive."""

    name: str
    archive_stem: str
    executable: str
    built_path: str


ROLES = (
    Role("service", "morphir-provider-snapshot", "morphir-provider-snapshot", "morphir-provider-snapshot"),
    Role(
        "test-fixture",
        "morphir-provider-snapshot-test-fixture",
        "provider-snapshot-fixture",
        "examples/provider-snapshot-fixture",
    ),
)


class ProviderReleaseError(ReleaseError):
    """Report an invalid provider release without a traceback."""


def archive_name(role: Role, version: str, target: str) -> str:
    """Return the published archive name for one role and target."""
    return f"{role.archive_stem}-{version}-{target}.tgz"


def descriptor_name(version: str) -> str:
    """Return the published descriptor name."""
    return f"morphir-provider-snapshot-{version}.release.json"


def fragment_name(version: str, target: str) -> str:
    """Return the per-target descriptor fragment name (not published)."""
    return f"morphir-provider-snapshot-{version}-{target}.provenance.json"


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def require_target(target: str) -> Target:
    selected = TARGETS.get(target)
    if selected is None:
        raise ProviderReleaseError(f"unsupported provider release target: {target}")
    return selected


def require_commit(commit: str) -> str:
    if COMMIT_PATTERN.fullmatch(commit) is None:
        raise ProviderReleaseError(f"invalid peeled Git commit: {commit}")
    return commit


def package_version(root: Path) -> str:
    """Return the morphir-package version, following workspace inheritance."""
    manifest = load_toml(root / "crates" / PACKAGE / "Cargo.toml").get("package")
    if not isinstance(manifest, Mapping) or manifest.get("name") != PACKAGE:
        raise ProviderReleaseError(f"crates/{PACKAGE}/Cargo.toml is not the {PACKAGE} package")
    version = manifest.get("version")
    if isinstance(version, Mapping) and version.get("workspace") is True:
        workspace = load_toml(root / "Cargo.toml").get("workspace")
        shared = workspace.get("package") if isinstance(workspace, Mapping) else None
        version = shared.get("version") if isinstance(shared, Mapping) else None
    if not isinstance(version, str):
        raise ProviderReleaseError(f"crates/{PACKAGE}/Cargo.toml has no version")
    return validate_semver(version, PACKAGE)


def resolve_tag(root: Path, tag: str) -> str:
    """Return the tag version after checking it equals the package version."""
    match = TAG_PATTERN.fullmatch(tag)
    if match is None:
        raise ProviderReleaseError(f"invalid tag {tag}; expected provider-snapshot/v<semver>")
    version = validate_semver(match.group("version"), "tag")
    expected = package_version(root)
    if version != expected:
        raise ProviderReleaseError(f"tag version {version} does not match {PACKAGE} version {expected}")
    return version


def require_commit_on_main(root: Path, commit: str, main_ref: str) -> None:
    """Fail unless the tagged commit is reachable from main."""
    result = subprocess.run(
        ["git", "-C", os.fspath(root), "merge-base", "--is-ancestor", require_commit(commit), main_ref],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode == 1:
        raise ProviderReleaseError(f"commit {commit} is not on main ({main_ref})")
    if result.returncode != 0:
        raise ProviderReleaseError(f"cannot check commit {commit}: {result.stderr.strip()}")


def deterministic_archive(executable: Path, name: str) -> bytes:
    """Return a gzip tar with one 0755 file, fixed metadata and no timestamps."""
    data = executable.read_bytes()
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = 0o755
    info.mtime = 0
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    tar_bytes = io.BytesIO()
    with tarfile.open(fileobj=tar_bytes, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        archive.addfile(info, io.BytesIO(data))
    compressed = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=compressed, mtime=0) as stream:
        stream.write(tar_bytes.getvalue())
    return compressed.getvalue()


def read_single_executable(archive: Path, expected: str) -> bytes:
    """Read the one regular file of an archive without extracting other members."""
    try:
        with tarfile.open(archive, mode="r:gz") as source:
            members = source.getmembers()
            if len(members) != 1:
                raise ProviderReleaseError(f"{archive.name} must contain exactly one file")
            member = members[0]
            if member.name != expected or not member.isreg():
                raise ProviderReleaseError(f"{archive.name} must contain the regular file {expected}")
            if member.mode & 0o777 != 0o755:
                raise ProviderReleaseError(f"{archive.name} member {expected} must have mode 0755")
            stream = source.extractfile(member)
            if stream is None:
                raise ProviderReleaseError(f"cannot read {expected} from {archive.name}")
            return stream.read()
    except (OSError, tarfile.TarError) as error:
        raise ProviderReleaseError(f"cannot read {archive.name}: {error}") from error


def write_checksum(path: Path) -> None:
    Path(f"{path}.sha256").write_text(f"{sha256_file(path)}  {path.name}\n", encoding="utf-8")


def check_checksum(path: Path) -> str:
    """Return the archive digest after comparing it with its .sha256 file."""
    checksum = Path(f"{path}.sha256")
    try:
        line = checksum.read_text(encoding="utf-8")
    except OSError as error:
        raise ProviderReleaseError(f"missing checksum for {path.name}: {error}") from error
    actual = sha256_file(path)
    if line != f"{actual}  {path.name}\n":
        raise ProviderReleaseError(f"checksum mismatch for {path.name}")
    return actual


def package(release_dir: Path, version: str, target: str, output: Path) -> None:
    """Write both archives and their checksums into a new output directory."""
    require_target(target)
    output.mkdir(parents=True, exist_ok=False)
    for role in ROLES:
        executable = release_dir / role.built_path
        if not executable.is_file():
            raise ProviderReleaseError(f"missing built executable {executable}")
        archive = output / archive_name(role, version, target)
        archive.write_bytes(deterministic_archive(executable, role.executable))
        write_checksum(archive)


def extract(assets: Path, version: str, target: str, output: Path) -> None:
    """Write each verified executable into a new directory for qualification."""
    output.mkdir(parents=True, exist_ok=False)
    for role in ROLES:
        archive = assets / archive_name(role, version, target)
        check_checksum(archive)
        destination = output / role.executable
        destination.write_bytes(read_single_executable(archive, role.executable))
        destination.chmod(stat.S_IRWXU | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH)


def parse_rustc(text: str) -> dict[str, str]:
    """Parse `rustc -vV` into its version line and its `key: value` fields."""
    lines = [line for line in text.splitlines() if line.strip()]
    if not lines or not lines[0].startswith("rustc "):
        raise ProviderReleaseError("rustc version output must start with 'rustc '")
    result = {"version": lines[0]}
    for line in lines[1:]:
        key, separator, value = line.partition(": ")
        if separator and key.strip() != "version":
            result[key.strip()] = value.strip()
    for key in ("release", "commit-hash", "host"):
        if key not in result:
            raise ProviderReleaseError(f"rustc version output has no {key}")
    return result


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ProviderReleaseError(f"cannot read {path}: {error}") from error


def read_qualification(evidence: Path) -> dict[str, Any]:
    """Return observed protocol facts after requiring every signed variant."""
    capabilities = load_json(evidence / "capabilities.json")
    variants = []
    for variant in VARIANTS:
        observed = load_json(evidence / f"variant-{variant}.json")
        if observed.get("variant") != variant:
            raise ProviderReleaseError(f"evidence for {variant} names another variant")
        variants.append(observed)
    return {"protocol": capabilities, "variants": variants}


def describe(
    assets: Path,
    version: str,
    commit: str,
    target: str,
    rustc: Path,
    evidence: Path,
    runner_image: str | None,
) -> Path:
    """Write the per-target fragment from verified archives and evidence."""
    selected = require_target(target)
    compiler = parse_rustc(rustc.read_text(encoding="utf-8"))
    if compiler["host"] != target:
        raise ProviderReleaseError(f"rustc host {compiler['host']} does not match target {target}")
    artifacts = []
    for role in ROLES:
        archive = assets / archive_name(role, version, target)
        executable = read_single_executable(archive, role.executable)
        artifacts.append(
            {
                "role": role.name,
                "filename": archive.name,
                "sha256": check_checksum(archive),
                "executable": role.executable,
                "executableSha256": sha256_bytes(executable),
                "target": target,
                "os": selected.os,
                "arch": selected.arch,
                "runner": selected.runner,
                "runnerImageVersion": runner_image,
                "buildMode": BUILD_MODE,
                "rustc": compiler,
            }
        )
    fragment = {
        "gitCommit": require_commit(commit),
        "version": version,
        "target": target,
        "artifacts": artifacts,
        "qualification": read_qualification(evidence),
    }
    path = assets / fragment_name(version, target)
    path.write_text(json.dumps(fragment, indent=2) + "\n", encoding="utf-8")
    return path


def assemble(assets: Path, tag: str, version: str, commit: str) -> Path:
    """Merge every target fragment into the single published descriptor."""
    require_commit(commit)
    artifacts: list[dict[str, Any]] = []
    protocol = None
    qualification = {}
    for target in sorted(TARGETS):
        fragment = load_json(assets / fragment_name(version, target))
        if fragment.get("gitCommit") != commit or fragment.get("version") != version:
            raise ProviderReleaseError(f"{target} fragment names another commit or version")
        if fragment.get("target") != target:
            raise ProviderReleaseError(f"{target} fragment names another target")
        observed = fragment["qualification"]
        if protocol is None:
            protocol = observed["protocol"]
        elif observed["protocol"] != protocol:
            raise ProviderReleaseError(f"{target} reports a different snapshot protocol")
        qualification[target] = [variant["variant"] for variant in observed["variants"]]
        for artifact in fragment["artifacts"]:
            archive = assets / artifact["filename"]
            if check_checksum(archive) != artifact["sha256"]:
                raise ProviderReleaseError(f"{archive.name} differs from its {target} fragment")
            artifacts.append(artifact)
    descriptor = {
        "schemaVersion": DESCRIPTOR_SCHEMA,
        "tag": tag,
        "package": PACKAGE,
        "version": version,
        "gitCommit": commit,
        "compatibility": protocol,
        "qualifiedFixtureVariants": qualification,
        "artifacts": artifacts,
    }
    path = assets / descriptor_name(version)
    path.write_text(json.dumps(descriptor, indent=2) + "\n", encoding="utf-8")
    return path


def verify(assets: Path, tag: str, version: str, commit: str, target: str) -> None:
    """Check downloaded archives against checksums and the published descriptor."""
    require_target(target)
    descriptor = load_json(assets / descriptor_name(version))
    expected = {"tag": tag, "package": PACKAGE, "version": version, "gitCommit": require_commit(commit)}
    for key, value in expected.items():
        if descriptor.get(key) != value:
            raise ProviderReleaseError(f"descriptor {key} is {descriptor.get(key)!r}, expected {value!r}")
    if descriptor.get("schemaVersion") != DESCRIPTOR_SCHEMA:
        raise ProviderReleaseError("unsupported descriptor schemaVersion")
    if descriptor.get("qualifiedFixtureVariants", {}).get(target) != list(VARIANTS):
        raise ProviderReleaseError(f"descriptor does not qualify every variant for {target}")
    entries = {(a["role"], a["target"]): a for a in descriptor.get("artifacts", [])}
    for role in ROLES:
        artifact = entries.get((role.name, target))
        if artifact is None:
            raise ProviderReleaseError(f"descriptor has no {role.name} artifact for {target}")
        archive = assets / archive_name(role, version, target)
        if artifact["filename"] != archive.name or check_checksum(archive) != artifact["sha256"]:
            raise ProviderReleaseError(f"{archive.name} does not match the descriptor")
        executable = read_single_executable(archive, role.executable)
        if sha256_bytes(executable) != artifact["executableSha256"]:
            raise ProviderReleaseError(f"{role.executable} digest does not match the descriptor")
        if artifact["buildMode"] != BUILD_MODE:
            raise ProviderReleaseError(f"{archive.name} is not a {BUILD_MODE} build")


def github_outputs(fields: Mapping[str, str]) -> None:
    destination = os.environ.get("GITHUB_OUTPUT")
    contents = "".join(f"{key}={value}\n" for key, value in fields.items())
    if destination is None:
        sys.stdout.write(contents)
        return
    with Path(destination).open("a", encoding="utf-8", newline="\n") as output:
        output.write(contents)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    resolve = commands.add_parser("resolve", help="validate a tag and its main ancestry")
    resolve.add_argument("--tag", required=True)
    resolve.add_argument("--commit", required=True)
    resolve.add_argument("--main-ref", default="origin/main")
    resolve.add_argument("--root", type=Path, default=Path.cwd())
    pack = commands.add_parser("package", help="archive built executables")
    pack.add_argument("--release-dir", type=Path, required=True)
    pack.add_argument("--version", required=True)
    pack.add_argument("--target", required=True)
    pack.add_argument("--output", type=Path, required=True)
    unpack = commands.add_parser("extract", help="extract verified executables")
    unpack.add_argument("--assets", type=Path, required=True)
    unpack.add_argument("--version", required=True)
    unpack.add_argument("--target", required=True)
    unpack.add_argument("--output", type=Path, required=True)
    fragment = commands.add_parser("describe", help="write one target fragment")
    fragment.add_argument("--assets", type=Path, required=True)
    fragment.add_argument("--version", required=True)
    fragment.add_argument("--commit", required=True)
    fragment.add_argument("--target", required=True)
    fragment.add_argument("--rustc", type=Path, required=True)
    fragment.add_argument("--evidence", type=Path, required=True)
    fragment.add_argument("--runner-image", default=os.environ.get("ImageVersion"))
    merge = commands.add_parser("assemble", help="write the release descriptor")
    merge.add_argument("--assets", type=Path, required=True)
    merge.add_argument("--tag", required=True)
    merge.add_argument("--version", required=True)
    merge.add_argument("--commit", required=True)
    check = commands.add_parser("verify", help="verify downloaded assets for one target")
    check.add_argument("--assets", type=Path, required=True)
    check.add_argument("--tag", required=True)
    check.add_argument("--version", required=True)
    check.add_argument("--commit", required=True)
    check.add_argument("--target", required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        if args.command == "resolve":
            version = resolve_tag(args.root.resolve(), args.tag)
            require_commit_on_main(args.root, args.commit, args.main_ref)
            github_outputs({"tag": args.tag, "version": version, "commit": args.commit})
        elif args.command == "package":
            package(args.release_dir, args.version, args.target, args.output)
        elif args.command == "extract":
            extract(args.assets, args.version, args.target, args.output)
        elif args.command == "describe":
            describe(
                args.assets,
                args.version,
                args.commit,
                args.target,
                args.rustc,
                args.evidence,
                args.runner_image,
            )
        elif args.command == "assemble":
            assemble(args.assets, args.tag, args.version, args.commit)
        else:
            verify(args.assets, args.tag, args.version, args.commit, args.target)
    except (ReleaseError, KeyError, TypeError) as error:
        print(f"provider release error: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
