//! The draft-3 local Library MVP testee over the shared MCK JSON-lines protocol.

use crate::package::positive_integer_id;
use anyhow::{Context, Result, bail, ensure};
use morphir_package::{digest::Digest, local_registry::mvp, strict_json};
use package_tough::{error, schema};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, Write},
    path::Path,
};

const PROFILE: &str = "local-library-mvp:0.1.0-draft.1";
const CONTRACT: &str = "0.1.0-draft.3";
const MAX_FILE: usize = 16_777_216;
const MAX_TOTAL: usize = 33_554_432;

#[derive(Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Request {
    #[serde(rename = "capabilities")]
    Capabilities {},
    #[serde(rename = "restore-local-library")]
    RestoreLocalLibrary {
        profile: String,
        files: Vec<WireFile>,
    },
    #[serde(rename = "exit")]
    Exit {},
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFile {
    path: String,
    hex: String,
}

/// Drive the bounded local Library MVP adapter through actual package APIs.
/// An invalid protocol or unclassified production error stops the process.
pub fn run(reader: impl BufRead, mut writer: impl Write) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut requests = 0usize;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        requests += 1;
        let mut value = strict_json::parse(&line)?;
        let id = value
            .as_object_mut()
            .and_then(|object| object.remove("id"))
            .filter(positive_integer_id)
            .ok_or_else(|| anyhow::anyhow!("package MVP request id must be a positive integer"))?;
        let request: Request = serde_json::from_value(value)?;
        let mut response = match request {
            Request::Capabilities {} => json!({
                "suite":"package","contractVersion":CONTRACT,
                "implementation":"morphir-rust","implementationVersion":env!("CARGO_PKG_VERSION"),
                "profiles":[PROFILE],"operations":["restore-local-library"]
            }),
            Request::RestoreLocalLibrary { profile, files } => {
                ensure!(profile == PROFILE, "unsupported package MVP profile");
                let files = admit_files(files)?;
                runtime.block_on(restore(files))?
            }
            Request::Exit {} => break,
        };
        response
            .as_object_mut()
            .expect("response is an object")
            .insert("id".into(), id);
        writeln!(writer, "{response}")?;
        writer.flush()?;
    }
    ensure!(requests > 0, "package MVP protocol input was empty");
    Ok(())
}

fn admit_files(files: Vec<WireFile>) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(
        files.len() == 15,
        "package MVP requires exactly 15 input files"
    );
    let mut found = BTreeMap::new();
    let mut total = 0usize;
    for file in files {
        ensure!(
            allowed_path(&file.path),
            "unsupported package MVP input path"
        );
        ensure!(
            file.hex.len() <= 2 * MAX_FILE,
            "package MVP input file limit"
        );
        let bytes = decode_hex(&file.hex)?;
        total = total
            .checked_add(bytes.len())
            .context("package MVP input size overflow")?;
        ensure!(total <= MAX_TOTAL, "package MVP total input limit");
        ensure!(
            found.insert(file.path, bytes).is_none(),
            "duplicate package MVP input path"
        );
    }
    for name in [
        "trust-policy.json",
        "morphir.lock",
        "registry/metadata/1.root.json",
        "registry/metadata/1.snapshot.json",
        "registry/metadata/1.targets.json",
        "registry/metadata/1.timestamp.json",
        "registry/metadata/timestamp.json",
    ] {
        ensure!(
            found.contains_key(name),
            "missing package MVP input file: {name}"
        );
    }
    let mut bundles = BTreeMap::<&str, BTreeSet<&str>>::new();
    let mut records = 0;
    let mut statements = 0;
    for name in found.keys() {
        if let Some(rest) = name.strip_prefix("registry/bundles/") {
            let (digest, leaf) = rest.split_once('/').context("invalid bundle path")?;
            bundles.entry(digest).or_default().insert(leaf);
        } else if name.starts_with("registry/targets/records/") {
            records += 1;
        } else if name.starts_with("registry/targets/statements/") {
            statements += 1;
        }
    }
    ensure!(
        bundles.len() == 2
            && bundles.values().all(|files| files.len() == 2
                && files.contains("ir.json")
                && files.contains("manifest.json")),
        "package MVP requires two complete bundles"
    );
    ensure!(
        records == 2 && statements == 2,
        "package MVP requires two records and statements"
    );
    Ok(found)
}

fn allowed_path(path: &str) -> bool {
    if matches!(
        path,
        "trust-policy.json"
            | "morphir.lock"
            | "registry/metadata/1.root.json"
            | "registry/metadata/1.snapshot.json"
            | "registry/metadata/1.targets.json"
            | "registry/metadata/1.timestamp.json"
            | "registry/metadata/timestamp.json"
    ) {
        return true;
    }
    if let Some(rest) = path.strip_prefix("registry/bundles/") {
        return rest.split_once('/').is_some_and(|(digest, leaf)| {
            digest.len() == 64
                && digest.bytes().all(lower_hex)
                && matches!(leaf, "manifest.json" | "ir.json")
        });
    }
    for prefix in ["registry/targets/records/", "registry/targets/statements/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return rest.split_once('.').is_some_and(|(digest, suffix)| {
                digest.len() == 64
                    && digest.bytes().all(lower_hex)
                    && suffix.ends_with(".json")
                    && !suffix.contains('/')
                    && !suffix.contains('\\')
                    && suffix.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'-' | b'.')
                    })
            });
        }
    }
    false
}

fn lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn decode_hex(hex: &str) -> Result<Vec<u8>> {
    ensure!(
        hex.len().is_multiple_of(2) && hex.bytes().all(lower_hex),
        "expected even-length lowercase hexadecimal bytes"
    );
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair)?, 16).map_err(Into::into))
        .collect()
}

async fn restore(files: BTreeMap<String, Vec<u8>>) -> Result<serde_json::Value> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    for (name, bytes) in &files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().context("input has no parent")?)?;
        fs::write(path, bytes)?;
    }
    let registry = root.join("registry");
    let before_registry = inventory(&registry)?;
    let lock = root.join("morphir.lock");
    let before_lock = fs::read(&lock)?;
    let policy = files.get("trust-policy.json").context("missing policy")?;
    let bootstrap = files
        .get("registry/metadata/1.root.json")
        .context("missing root")?;
    let state = root.join("trust");
    let output = root.join("output");
    mvp::initialize(mvp::InitializeRequest {
        policy,
        root: bootstrap,
        state: &state,
    })?;
    let result = mvp::restore(mvp::RestoreRequest {
        policy,
        lock: &before_lock,
        registry: &registry,
        state: &state,
        output: &output,
    })
    .await;
    let lock_unchanged = fs::read(&lock)? == before_lock;
    let registry_unchanged = inventory(&registry)? == before_registry;
    ensure!(
        lock_unchanged && registry_unchanged,
        "restore changed package MVP inputs"
    );
    match result {
        Ok(report) => {
            ensure!(
                output.is_dir(),
                "restore did not publish an output directory"
            );
            let output_files = inventory(&output)?
                .into_iter()
                .filter_map(|(path, bytes)| {
                    bytes.map(
                        |bytes| json!({"path":path,"sha256":Digest::of_bytes(&bytes).to_string()}),
                    )
                })
                .collect::<Vec<_>>();
            let mut restored = report.packages;
            restored.sort_by(|left, right| {
                left.release
                    .package_path()
                    .cmp(right.release.package_path())
            });
            let packages = restored
                .into_iter()
                .map(|package| {
                    let release = package.release;
                    ensure!(
                        output.join(&package.directory).is_dir(),
                        "restored package directory is absent"
                    );
                    Ok(json!({
                        "packagePath": release.package_path().as_str(),
                        "version": release.version().as_str(),
                        "directory": package.directory
                    }))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(
                json!({"outcome":"restored","packages":packages,"outputFiles":output_files,"output":"present","lockUnchanged":lock_unchanged,"registryUnchanged":registry_unchanged}),
            )
        }
        Err(error) if timestamp_signature_failure(&error) => {
            ensure!(!output.exists(), "refused restore published output");
            Ok(
                json!({"outcome":"refused","category":"metadata-authentication","reason":"timestamp-signature-threshold","output":"absent","lockUnchanged":lock_unchanged,"registryUnchanged":registry_unchanged}),
            )
        }
        Err(error) => bail!("package MVP restore failed: {error:#}"),
    }
}

fn timestamp_signature_failure(error: &mvp::Error) -> bool {
    matches!(error,
        mvp::Error::Metadata(morphir_package::local_registry::tuf::MetadataLoadError::Update(source))
            if matches!(source.as_ref(),
                error::Error::VerifyMetadata { role: schema::RoleType::Timestamp,
                    source: schema::Error::SignatureThreshold { .. }, .. })
    )
}

fn inventory(root: &Path) -> Result<BTreeMap<String, Option<Vec<u8>>>> {
    fn visit(
        root: &Path,
        directory: &Path,
        found: &mut BTreeMap<String, Option<Vec<u8>>>,
    ) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let name = path
                .strip_prefix(root)?
                .to_str()
                .context("non-UTF8 registry entry")?
                .replace('\\', "/");
            let ty = entry.file_type()?;
            if ty.is_dir() {
                found.insert(name, None);
                visit(root, &path, found)?;
            } else if ty.is_file() {
                found.insert(name, Some(fs::read(&path)?));
            } else {
                bail!("unexpected registry file type");
            }
        }
        Ok(())
    }
    let mut found = BTreeMap::new();
    visit(root, root, &mut found)?;
    Ok(found)
}
