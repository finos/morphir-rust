use super::{Error, RestoredPackage, files, require, store::Backend};
use crate::{
    library::{LibraryInput, VerifiedLibrarySet},
    local_registry::*,
    metadata::NormalizedMetadata,
    schema::PackageSchemas,
};
use serde_json::{Value, json};
use std::path::Path;
fn digest(bytes: &[u8]) -> String {
    crate::digest::Digest::of_bytes(bytes).to_string()
}
pub(super) fn target(
    registry: &Path,
    path: &RegistryPath,
    pin: &Digest,
    targets: &Value,
    release: &crate::resolution::ReleaseId,
    record: bool,
) -> Result<Vec<u8>, Error> {
    let target = targets
        .get(path.as_str())
        .ok_or(Error::Refused("locked target absent from fresh repository"))?;
    let hash = target["hashes"]["sha256"]
        .as_str()
        .ok_or(Error::Refused("target hash"))?;
    require(
        pin.as_str() == format!("sha256:{hash}"),
        "target hash differs from lock",
    )?;
    let length = target["length"]
        .as_u64()
        .ok_or(Error::Refused("target length"))?;
    require(length <= 1_048_576, "target resource limit")?;
    if record {
        let authority = &target["custom"]["morphir"];
        require(
            authority["formatVersion"] == "0.1.0-draft.3"
                && authority["kind"] == "LibraryRelease"
                && authority["status"] == "active"
                && authority["release"] == serde_json::to_value(release)?,
            "release is not active in fresh repository",
        )?;
    }
    let (parent, file) = path
        .as_str()
        .rsplit_once('/')
        .ok_or(Error::Refused("target path"))?;
    let physical = format!("targets/{parent}/{hash}.{file}");
    let bytes = files::read(registry, &physical, 1_048_576)?;
    require(
        bytes.len() as u64 == length && digest(&bytes) == pin.as_str(),
        "target hash or length mismatch",
    )?;
    if let Some(expected) = target["hashes"].get("sha512") {
        use sha2::Digest as _;
        let actual: String = sha2::Sha512::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        require(
            expected.as_str() == Some(actual.as_str()),
            "target SHA512 mismatch",
        )?;
    }
    Ok(bytes)
}
pub(super) fn graph(
    backend: &Backend,
    registry: &Path,
    policy: &TrustPolicy,
    lock: &LibraryLock,
    targets: &Value,
    stage: &Path,
) -> Result<Vec<RestoredPackage>, Error> {
    let schemas = PackageSchemas::compile(
        &serde_json::from_str(include_str!("schemas/library-manifest.schema.json"))?,
        &serde_json::from_str(include_str!("schemas/lock-core.schema.json"))?,
    )?;
    let mut libraries = vec![];
    let mut packages = vec![];
    let mut nodes = serde_json::Map::new();
    let graph = lock.graph();
    let index = |release: &crate::resolution::ReleaseId| {
        graph
            .nodes()
            .iter()
            .position(|n| n.release() == release)
            .map(|i| format!("n{i}"))
            .ok_or(Error::Refused("missing locked binding"))
    };
    let mut content_total = 0usize;
    for (i, node) in graph.nodes().iter().enumerate() {
        let release = node.release();
        require(
            repository_permits(
                policy,
                policy.repositories()[0].identity(),
                release.package_path(),
            ),
            "repository namespace unauthorized",
        )?;
        let acquisition = lock
            .acquisitions()
            .iter()
            .find(|a| a.release() == release)
            .ok_or(Error::Refused("missing acquisition"))?;
        let subject = ObjectSubject {
            registry: acquisition.registry().clone(),
            path: acquisition.record().path().clone(),
        };
        let record_bytes = target(
            registry,
            acquisition.record().path(),
            acquisition.record().digest(),
            targets,
            release,
            true,
        )?;
        backend.charge(record_bytes.len())?;
        let record = decode_registry_record(&record_bytes, &Subject::from(&subject))?;
        let statement = lock
            .evidence()
            .iter()
            .find(|e| e.id() == acquisition.statement())
            .ok_or(Error::Refused("missing publisher evidence"))?;
        require(
            record.release_record().release() == release
                && record.source() == acquisition.source()
                && record.statement() == statement.reference(),
            "record acquisition mismatch",
        )?;
        let statement_bytes = target(
            registry,
            statement.reference().path(),
            statement.reference().digest(),
            targets,
            release,
            false,
        )?;
        backend.charge(statement_bytes.len())?;
        let subject = ObjectSubject {
            registry: statement.registry().clone(),
            path: statement.reference().path().clone(),
        };
        let envelope = prepare_publisher_envelope(&statement_bytes, &subject)?;
        let evidence = verify_publisher_signatures(&envelope, release, policy)?;
        let published =
            decode_release_statement(evidence.payload_bytes(), &Subject::from(&subject))?;
        require(
            &published == record.release_record(),
            "publisher statement differs from record",
        )?;
        require(
            published.ir_package_name() == node.ir_package_name()
                && published.manifest_digest() == node.manifest_digest()
                && published.content_digest() == node.content_digest(),
            "record differs from locked node",
        )?;
        let source = acquisition.source().path().as_str();
        let bytes = files::read(registry, &format!("{source}/manifest.json"), 1_048_576)?;
        backend.charge(bytes.len())?;
        let manifest = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Refused("manifest UTF-8"))?
            .to_owned();
        let normalized = NormalizedMetadata::parse(&manifest)?;
        require(
            normalized.manifest_digest().to_string() == node.manifest_digest().as_str()
                && normalized.content_digest().to_string() == node.content_digest().as_str(),
            "manifest differs from locked digest",
        )?;
        let dependencies = normalized.value()["dependencies"]
            .as_object()
            .ok_or(Error::Refused("manifest dependencies"))?;
        require(
            dependencies.len() == published.dependencies().len(),
            "record dependency mismatch",
        )?;
        for requirement in published.dependencies() {
            let value = dependencies
                .get(requirement.ir_package_name().as_str())
                .ok_or(Error::Refused("record dependency missing"))?;
            require(
                value["packagePath"] == requirement.package_path().as_str()
                    && value["versionRange"] == serde_json::to_value(requirement.version_range())?,
                "record dependency mismatch",
            )?;
        }
        let content = normalized.value()["content"]
            .as_object()
            .ok_or(Error::Refused("manifest content"))?;
        require(content.len() <= 4096, "package file count limit")?;
        let expected = content
            .keys()
            .cloned()
            .chain(std::iter::once("manifest.json".into()))
            .collect();
        require(
            files::inventory(&registry.join(source))? == expected,
            "bundle inventory differs from manifest",
        )?;
        let directory = format!(
            "{}/{}",
            release.package_path().as_str(),
            release.version().as_str()
        );
        RegistryPath::parse(&directory).map_err(|_| Error::Refused("unsafe release directory"))?;
        let mut files_in = vec![];
        for path in content.keys() {
            let bytes = files::read(registry, &format!("{source}/{path}"), 67_108_864)?;
            require(
                content[path].as_str() == Some(digest(&bytes).as_str()),
                "content digest mismatch",
            )?;
            content_total = content_total
                .checked_add(bytes.len())
                .ok_or(Error::Refused("graph content limit"))?;
            require(content_total <= 268_435_456, "graph content limit")?;
            files::write(stage, &format!("{directory}/{path}"), &bytes)?;
            files_in.push((path.clone(), bytes));
        }
        files::write(stage, &format!("{directory}/manifest.json"), &bytes)?;
        require(
            files::inventory(&registry.join(source))? == expected,
            "bundle inventory changed while copying",
        )?;
        libraries.push(LibraryInput::new(manifest, files_in));
        packages.push(RestoredPackage {
            release: release.clone(),
            directory,
        });
        let bindings = node
            .bindings()
            .iter()
            .map(|b| {
                Ok((
                    b.ir_package_name().as_str().into(),
                    Value::String(index(b.target())?),
                ))
            })
            .collect::<Result<serde_json::Map<_, _>, Error>>()?;
        nodes.insert(format!("n{i}"),json!({"release":release,"irPackageName":node.ir_package_name(),"manifestDigest":node.manifest_digest(),"contentDigest":node.content_digest(),"bindings":bindings}));
    }
    let lock_core = json!({"formatVersion":"0.1.0-draft.1","kind":"LibraryLockCore","root":index(graph.root())?,"nodes":nodes});
    VerifiedLibrarySet::verify(&schemas, &serde_json::to_string(&lock_core)?, &libraries)?;
    Ok(packages)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_checks_every_advertised_supported_hash() {
        let root = tempfile::tempdir().unwrap();
        let bytes = b"publisher bytes";
        let digest = digest(bytes);
        let hash = digest.strip_prefix("sha256:").unwrap();
        std::fs::create_dir_all(root.path().join("targets/statements")).unwrap();
        std::fs::write(
            root.path()
                .join(format!("targets/statements/{hash}.a.json")),
            bytes,
        )
        .unwrap();
        let targets = json!({"statements/a.json":{"length":bytes.len(),"hashes":{"sha256":hash,"sha512":"00".repeat(64)}}});
        let release = crate::resolution::ReleaseId::new(
            crate::resolution::PackagePath::parse("example.com/a").unwrap(),
            crate::resolution::StableVersion::parse("1.0.0").unwrap(),
        );
        assert!(
            target(
                root.path(),
                &RegistryPath::parse("statements/a.json").unwrap(),
                &Digest::parse(&digest).unwrap(),
                &targets,
                &release,
                false
            )
            .is_err()
        );
    }
}
