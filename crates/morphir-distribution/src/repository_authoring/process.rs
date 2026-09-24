//! Verification and index conversion for per-artifact bundles.

use super::*;
use crate::Platform;
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionType, claims::CapabilityClaimSet};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn verify(
    root: &Path,
    entries: &BTreeMap<String, Vec<u8>>,
    descriptor: ReleaseBundleDescriptor,
    probe: &mut impl FnMut(&BundleArtifactDescriptor, &[u8]) -> Result<PublicationDescription>,
) -> Result<VerifiedReleaseBundle> {
    let runtime = descriptor.artifacts()[0].runtime();
    if descriptor
        .artifacts()
        .iter()
        .any(|artifact| artifact.runtime() != runtime)
    {
        return Err(invalid_bundle(
            root,
            "a bundle holds either WASM artifacts or process artifacts, not both",
        ));
    }
    let wire = serde_json::to_value(&descriptor).map_err(DistributionError::StateEncoding)?;
    let critical = record_critical(root, &wire)?;
    let mut names = BTreeSet::from(["release.json".to_owned()]);
    let mut platforms = BTreeSet::new();
    let mut artifacts = Vec::new();
    let mut records = Vec::new();
    let first = descriptor.artifacts()[0].claims();
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
        validate_claims(root, artifact.claims(), &descriptor)?;
        if wire["artifacts"][index].get("requires").is_some() {
            return Err(invalid_bundle(
                root,
                format!(
                    "artifact '{filename}': put host requirements in claims.requires, not artifact.requires"
                ),
            ));
        }
        if descriptor.platform_differences() != Some(PlatformDifferences::Declared) {
            let mut differences = Vec::new();
            differing_members(
                &wire["artifacts"][0]["claims"],
                &wire["artifacts"][index]["claims"],
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
            "claims": wire["artifacts"][index]["claims"], "claimCheck": "unchecked",
            "executable": artifact.runtime() == ArtifactRuntime::Process,
        });
        if let Some(platform) = platform {
            record["platform"] = json!(platform);
        }
        if let Some(critical) = wire["artifacts"][index].get("critical") {
            record["critical"] = critical.clone();
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
                    validate_claims(&path, &reported, &descriptor)?;
                    check_agreement(&path, artifact.claims(), &reported)?;
                    record["probeSource"] = json!("describe");
                }
                PublicationDescription::SessionFallback {
                    protocol_version,
                    extension,
                    capabilities,
                } => {
                    artifact
                        .claims()
                        .check_session(&protocol_version, &extension, &capabilities)
                        .map_err(|error| {
                            invalid_bundle(&path, format!("described session disagrees: {error}"))
                        })?;
                    record["probeSource"] = json!("session-fallback");
                }
            }
            record["claimCheck"] = json!("probed");
        }
    }
    let channel = if descriptor.version().pre.is_empty() {
        "stable"
    } else {
        "preview"
    };
    let mut release = json!({
        "schemaVersion": "2.0.0-draft.2", "id": descriptor.extension_id(),
        "name": first.extension.name, "version": descriptor.version(), "channels": [channel],
        "artifacts": records,
        "critical": critical,
    });
    if let Some(requires) = wire.get("requires") {
        release["requires"] = requires.clone();
    }
    let release =
        serde_json::from_value(release).map_err(|error| invalid_bundle(root, error.to_string()))?;
    Ok(VerifiedReleaseBundle { release, artifacts })
}

fn record_critical(root: &Path, descriptor: &Value) -> Result<Vec<String>> {
    let paths: Vec<String> = descriptor
        .get("critical")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(DistributionError::StateEncoding)?
        .unwrap_or_default();
    paths
        .into_iter()
        .map(|path| match path.as_str() {
            "extensionId" => Ok("id".into()),
            "shortId"
            | "gitCommit"
            | "platformDifferences"
            | "artifacts.requires"
            | "artifacts.requires.host" => Err(invalid_bundle(
                root,
                format!(
                    "critical path '{path}' has no corresponding member in the published record"
                ),
            )),
            _ => Ok(path),
        })
        .collect()
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
        "unknown-linux-gnu" => "linux",
        "pc-windows-msvc" => "windows",
        _ => {
            return Err(invalid_bundle(
                root,
                format!(
                    "unsupported process platform '{triple}': this index cannot represent it yet"
                ),
            ));
        }
    };
    let arch = match arch {
        "arm64" | "aarch64" => "aarch64",
        "i686" | "i586" | "i386" | "x86" => "x86",
        "amd64" | "x86_64" => "x86_64",
        "powerpc64le" if os == "linux" => "powerpc64",
        "riscv64gc" if os == "linux" => "riscv64",
        "loongarch64" if os == "linux" => "loongarch64",
        _ => {
            return Err(invalid_bundle(
                root,
                format!(
                    "unsupported process architecture '{arch}' in '{triple}': this index cannot represent it yet"
                ),
            ));
        }
    };
    Platform::new(os, arch)
}

fn validate_claims(
    root: &Path,
    claims: &CapabilityClaimSet,
    descriptor: &ReleaseBundleDescriptor,
) -> Result<()> {
    if claims.extension.id != descriptor.extension_id().as_str()
        || claims.extension.version != descriptor.version().to_string()
    {
        return Err(invalid_bundle(
            root,
            "claims extension.id or extension.version differs from release.json",
        ));
    }
    if claims.protocol_versions.is_empty()
        || claims
            .protocol_versions
            .iter()
            .any(|version| version.trim().is_empty())
    {
        return Err(invalid_bundle(
            root,
            "claims protocolVersions must contain non-empty versions",
        ));
    }
    let kinds = &claims.extension.types;
    let mut seen = Vec::new();
    for kind in kinds {
        if seen.contains(kind) {
            return Err(invalid_bundle(
                root,
                "claims extension.types contains a repeated capability kind",
            ));
        }
        seen.push(*kind);
    }
    for (kind, member) in [
        (ExtensionType::Frontend, "frontend"),
        (ExtensionType::Backend, "backend"),
        (ExtensionType::Workspace, "workspace"),
    ] {
        if kinds.contains(&kind) != claims.capabilities.contains_key(member) {
            return Err(invalid_bundle(
                root,
                format!("claims extension.types and capabilities.{member} disagree"),
            ));
        }
    }
    // Unknown optional capability members remain on the wire, but known kinds
    // must have valid objects even when their artifact cannot run on this host.
    let known = ["frontend", "backend", "workspace"]
        .into_iter()
        .filter_map(|member| {
            claims
                .capabilities
                .get(member)
                .map(|value| (member.to_owned(), value.clone()))
        })
        .collect();
    serde_json::from_value::<ExtensionCapabilities>(Value::Object(known))
        .map_err(|error| invalid_bundle(root, format!("invalid claims capabilities: {error}")))?;
    Ok(())
}

fn check_agreement(
    path: &Path,
    stated: &CapabilityClaimSet,
    reported: &CapabilityClaimSet,
) -> Result<()> {
    // Reuse the SDK agreement rule in both directions: describe compares two
    // claim sets, so the session rule alone would allow omitted capabilities.
    for (expected, actual) in [(stated, reported), (reported, stated)] {
        for version in &actual.protocol_versions {
            if let Err(error) =
                expected.check_session(version, &actual.extension, &actual.capabilities)
            {
                return Err(invalid_bundle(
                    path,
                    format!(
                        "described claims disagree: {error}; differing members: {}",
                        claims_differences(stated, reported)?.join(", ")
                    ),
                ));
            }
        }
    }
    let differences = claims_differences(stated, reported)?;
    if differences.is_empty() {
        Ok(())
    } else {
        Err(invalid_bundle(
            path,
            format!("described claims disagree at {}", differences.join(", ")),
        ))
    }
}

fn claims_differences(
    left: &CapabilityClaimSet,
    right: &CapabilityClaimSet,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_reader_refuses_published_critical_claims_member() {
        let root = tempfile::tempdir().unwrap();
        let digest = Sha256Digest::of_bytes(b"wasm");
        let descriptor = serde_json::from_value(json!({
            "schemaVersion": "2.0.0-draft.2", "extensionId": "example", "shortId": "example",
            "version": "1.0.0", "critical": ["artifacts.claims.capabilities.backend.generate"],
            "artifacts": [{"runtime": "wasm", "filename": "guest.wasm", "sha256": digest,
                "claims": {"claimsVersion": "0.1.0-draft.2",
                    "protocolVersions": [morphir_extension_sdk::protocol::MEP_VERSION],
                    "extension": {"id": "example", "name": "Example", "version": "1.0.0", "types": ["backend"]},
                    "capabilities": {"backend": {"targets": ["sql"], "irVersions": ["3"], "generate": true}}
                }}]
        })).unwrap();
        let entries = BTreeMap::from([
            ("release.json".into(), vec![]),
            ("guest.wasm".into(), b"wasm".to_vec()),
            (
                "guest.wasm.sha256".into(),
                format!("{digest}  guest.wasm\n").into_bytes(),
            ),
        ]);
        let bundle = verify(root.path(), &entries, descriptor, &mut |_, _| {
            panic!("WASM must not probe")
        })
        .unwrap();
        let mut record = serde_json::to_value(bundle.release).unwrap();
        // Use the shared must-ignore reader with an older vocabulary that does
        // not understand per-artifact claim sets. Schema support is independent.
        let older_paths = [
            "schemaVersion",
            "id",
            "name",
            "version",
            "artifacts",
            "critical",
        ];
        let error =
            crate::extension_format::validate_members(&record, &older_paths, true).unwrap_err();
        assert!(
            error.contains("artifacts.claims.capabilities.backend.generate"),
            "{error}"
        );
        record.as_object_mut().unwrap().remove("critical");
        assert!(crate::extension_format::validate_members(&record, &older_paths, true).is_ok());
    }
}
