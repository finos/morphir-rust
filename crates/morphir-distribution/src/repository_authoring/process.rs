//! Verification and index conversion for per-artifact bundles.

use super::*;
use crate::Platform;
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionType, statement::CapabilityStatement};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn verify(
    root: &Path,
    entries: &BTreeMap<String, Vec<u8>>,
    descriptor: ReleaseBundleDescriptor,
    probe: &mut impl FnMut(&BundleArtifactDescriptor, &[u8]) -> Result<PublicationDescription>,
) -> Result<VerifiedReleaseBundle> {
    let wire = serde_json::to_value(&descriptor).map_err(DistributionError::StateEncoding)?;
    let mut names = BTreeSet::from(["release.json".to_owned()]);
    let mut platforms = BTreeSet::new();
    let mut artifacts = Vec::new();
    let mut records = Vec::new();
    let first = descriptor.artifacts()[0].statement();
    for (index, artifact) in descriptor.artifacts().iter().enumerate() {
        let filename = artifact.filename().as_str();
        if [".tgz", ".tar.gz", ".zip", ".tar"]
            .iter()
            .any(|suffix| filename.to_ascii_lowercase().ends_with(suffix))
        {
            return Err(invalid_bundle(
                root.join(filename),
                format!("publish supports raw executables only; `{filename}` is an archive"),
            ));
        }
        let checksum = format!("{filename}.sha256");
        if !names.insert(filename.to_owned()) || !names.insert(checksum.clone()) {
            return Err(invalid_bundle(
                root,
                format!("duplicate artifact filename '{filename}'"),
            ));
        }
        let platform = artifact
            .platform()
            .map(|triple| platform(root, triple))
            .transpose()?;
        if !platforms.insert(platform.clone()) {
            return Err(invalid_bundle(root, "duplicate artifact platform"));
        }
        validate_statement(root, artifact.statement(), &descriptor)?;
        if wire["artifacts"][index].get("requires").is_some() {
            return Err(invalid_bundle(
                root,
                format!(
                    "artifact '{filename}': put host requirements in statement.requires, not artifact.requires"
                ),
            ));
        }
        if descriptor.platform_differences() != Some(PlatformDifferences::Declared) {
            let mut differences = Vec::new();
            differing_members(
                &wire["artifacts"][0]["statement"],
                &wire["artifacts"][index]["statement"],
                "",
                &mut differences,
            );
            if !differences.is_empty() {
                return Err(invalid_bundle(
                    root,
                    format!(
                        "artifact '{filename}' differs at {}; set platformDifferences to 'declared' for intentional differences",
                        differences.join(", ")
                    ),
                ));
            }
        }
        let bytes = entries
            .get(filename)
            .ok_or_else(|| invalid_bundle(root.join(filename), "missing bundle artifact"))?;
        verify_digest(&root.join(filename), bytes, artifact.sha256())?;
        let expected = format!("{}  {filename}\n", artifact.sha256());
        if entries.get(&checksum).map(Vec::as_slice) != Some(expected.as_bytes()) {
            return Err(invalid_bundle(
                root.join(checksum),
                "release bundle checksum file does not match its artifact",
            ));
        }
        let mut record = json!({
            "runtime": artifact.runtime(), "filename": filename, "sha256": artifact.sha256(),
            "source": {"kind": "local-file", "path": format!("artifacts/{filename}")},
            "statement": wire["artifacts"][index]["statement"], "statementSource": "declared",
            "executable": artifact.runtime() == ArtifactRuntime::Process,
        });
        if let Some(platform) = platform {
            record["platform"] = json!(platform);
        }
        records.push(record);
        artifacts.push(VerifiedBundleArtifact {
            artifact: artifact.filename().clone(),
            artifact_bytes: bytes.clone(),
            digest: artifact.sha256().clone(),
        });
    }
    if entries.keys().cloned().collect::<BTreeSet<_>>() != names {
        return Err(invalid_bundle(
            root,
            "release bundle files do not match release.json",
        ));
    }

    // No guest runs until every digest, checksum and declaration has passed.
    for ((artifact, verified), record) in descriptor
        .artifacts()
        .iter()
        .zip(&artifacts)
        .zip(&mut records)
    {
        if artifact.runtime() == ArtifactRuntime::Process
            && record["platform"] == json!(Platform::current())
        {
            let path = root.join(artifact.filename().as_str());
            match probe(artifact, &verified.artifact_bytes)? {
                PublicationDescription::Describe(reported) => {
                    validate_statement(&path, &reported, &descriptor)?;
                    check_agreement(&path, artifact.statement(), &reported)?;
                    record["probeSource"] = json!("describe");
                }
                PublicationDescription::SessionFallback {
                    protocol_version,
                    extension,
                    capabilities,
                } => {
                    artifact
                        .statement()
                        .check_session(&protocol_version, &extension, &capabilities)
                        .map_err(|error| {
                            invalid_bundle(&path, format!("described session disagrees: {error}"))
                        })?;
                    record["probeSource"] = json!("session-fallback");
                }
            }
            record["statementSource"] = json!("probed");
        }
    }
    let channel = if descriptor.version().pre.is_empty() {
        "stable"
    } else {
        "preview"
    };
    let mut release = json!({
        "schemaVersion": "2.0.0-draft.1", "id": descriptor.extension_id(),
        "name": first.extension.name, "version": descriptor.version(), "channels": [channel],
        "artifacts": records,
    });
    if let Some(requires) = wire.get("requires") {
        release["requires"] = requires.clone();
        if requires.get("host").is_some() {
            release["critical"] = json!(["requires.host"]);
        }
    }
    let release =
        serde_json::from_value(release).map_err(|error| invalid_bundle(root, error.to_string()))?;
    Ok(VerifiedReleaseBundle { release, artifacts })
}

fn platform(root: &Path, triple: &str) -> Result<Platform> {
    let (arch, suffix) = triple.split_once('-').ok_or_else(|| {
        invalid_bundle(
            root,
            format!("invalid process platform '{triple}': expected a target triple"),
        )
    })?;
    let os = match suffix {
        "apple-darwin" => "macos",
        "unknown-linux-gnu" | "unknown-linux-musl" => "linux",
        "pc-windows-msvc" | "pc-windows-gnu" => "windows",
        _ => {
            return Err(invalid_bundle(
                root,
                format!("unsupported process platform '{triple}'"),
            ));
        }
    };
    let arch = match arch {
        "arm64" | "aarch64" => "aarch64",
        "i686" | "i586" | "i386" | "x86" => "x86",
        "amd64" | "x86_64" => "x86_64",
        "arm" | "armv7" | "armv7a" | "thumbv7neon" => "arm",
        "powerpc64" | "powerpc64le" => "powerpc64",
        "riscv64" | "riscv64gc" => "riscv64",
        "riscv32" | "riscv32gc" | "riscv32imac" => "riscv32",
        "loongarch64" | "s390x" | "sparc64" | "powerpc" => arch,
        _ => {
            return Err(invalid_bundle(
                root,
                format!("unsupported process architecture '{arch}'"),
            ));
        }
    };
    Platform::new(os, arch)
}

fn validate_statement(
    root: &Path,
    statement: &CapabilityStatement,
    descriptor: &ReleaseBundleDescriptor,
) -> Result<()> {
    if statement.extension.id != descriptor.extension_id().as_str()
        || statement.extension.version != descriptor.version().to_string()
    {
        return Err(invalid_bundle(
            root,
            "statement extension.id or extension.version differs from release.json",
        ));
    }
    if statement.protocol_versions.is_empty()
        || statement
            .protocol_versions
            .iter()
            .any(|version| version.trim().is_empty())
    {
        return Err(invalid_bundle(
            root,
            "statement protocolVersions must contain non-empty versions",
        ));
    }
    let kinds = &statement.extension.types;
    let mut seen = Vec::new();
    for kind in kinds {
        if seen.contains(kind) {
            return Err(invalid_bundle(
                root,
                "statement extension.types contains a repeated capability kind",
            ));
        }
        seen.push(*kind);
    }
    for (kind, member) in [
        (ExtensionType::Frontend, "frontend"),
        (ExtensionType::Backend, "backend"),
        (ExtensionType::Workspace, "workspace"),
    ] {
        if kinds.contains(&kind) != statement.capabilities.contains_key(member) {
            return Err(invalid_bundle(
                root,
                format!("statement extension.types and capabilities.{member} disagree"),
            ));
        }
    }
    // Unknown optional capability members remain on the wire, but known kinds
    // must have valid objects even when their artifact cannot run on this host.
    let known = ["frontend", "backend", "workspace"]
        .into_iter()
        .filter_map(|member| {
            statement
                .capabilities
                .get(member)
                .map(|value| (member.to_owned(), value.clone()))
        })
        .collect();
    serde_json::from_value::<ExtensionCapabilities>(Value::Object(known)).map_err(|error| {
        invalid_bundle(root, format!("invalid statement capabilities: {error}"))
    })?;
    Ok(())
}

fn check_agreement(
    path: &Path,
    stated: &CapabilityStatement,
    reported: &CapabilityStatement,
) -> Result<()> {
    // Reuse the SDK agreement rule in both directions: describe compares two
    // statements, so the session rule alone would allow omitted capabilities.
    for (expected, actual) in [(stated, reported), (reported, stated)] {
        for version in &actual.protocol_versions {
            if let Err(error) =
                expected.check_session(version, &actual.extension, &actual.capabilities)
            {
                return Err(invalid_bundle(
                    path,
                    format!(
                        "described statement disagrees: {error}; differing members: {}",
                        statement_differences(stated, reported)?.join(", ")
                    ),
                ));
            }
        }
    }
    let differences = statement_differences(stated, reported)?;
    if differences.is_empty() {
        Ok(())
    } else {
        Err(invalid_bundle(
            path,
            format!(
                "described statement disagrees at {}",
                differences.join(", ")
            ),
        ))
    }
}

fn statement_differences(
    left: &CapabilityStatement,
    right: &CapabilityStatement,
) -> Result<Vec<String>> {
    let left = serde_json::to_value(left).map_err(DistributionError::StateEncoding)?;
    let right = serde_json::to_value(right).map_err(DistributionError::StateEncoding)?;
    let mut differences = Vec::new();
    differing_members(&left, &right, "", &mut differences);
    Ok(differences)
}

fn differing_members(left: &Value, right: &Value, path: &str, differences: &mut Vec<String>) {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            for key in left.keys().chain(right.keys()).collect::<BTreeSet<_>>() {
                let path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => differing_members(left, right, &path, differences),
                    _ => differences.push(path),
                }
            }
        }
        _ if left != right => differences.push(path.to_owned()),
        _ => {}
    }
}
