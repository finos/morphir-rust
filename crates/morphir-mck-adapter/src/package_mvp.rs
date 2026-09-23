//! The draft-3 local Library MVP testee over the shared MCK JSON-lines protocol.

mod refresh;
mod update;

use crate::package::positive_integer_id;
use anyhow::{Context, Result, bail, ensure};
use morphir_package::{
    digest::Digest,
    local_registry::{Code, Phase, mvp, tuf},
    resolution::{PackagePath, ReleaseId, StableVersion, UpdateTarget},
    strict_json,
};
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
const MAX_REQUEST_LINE: usize = 2 * MAX_TOTAL + 65_536;
const EXTRA_BUNDLE_INPUT: &str = "registry/bundles/5922bc8860f6cd008b9cda341be7f3a776ea332e63261392e94c17e19a647886/undeclared.txt";

#[derive(Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Request {
    #[serde(rename = "capabilities")]
    Capabilities {},
    #[serde(rename = "restore-local-library")]
    RestoreLocalLibrary {
        profile: String,
        #[serde(default)]
        environment: Environment,
        files: Vec<WireFile>,
    },
    #[serde(rename = "resolve-local-library")]
    ResolveLocalLibrary {
        profile: String,
        #[serde(rename = "exactRoot")]
        exact_root: String,
        #[serde(default)]
        environment: Environment,
        files: Vec<WireFile>,
    },
    #[serde(rename = "refresh-local-library")]
    RefreshLocalLibrary {
        profile: String,
        #[serde(default)]
        environment: Environment,
        files: Vec<WireFile>,
    },
    #[serde(rename = "update-local-library")]
    UpdateLocalLibrary {
        profile: String,
        targets: Vec<String>,
        #[serde(default)]
        environment: Environment,
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

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Environment {
    #[serde(default)]
    trust_state: TrustState,
    #[serde(default)]
    output: OutputSetup,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum TrustState {
    #[default]
    Initialized,
    Uninitialized,
    MissingDatabase,
    CorruptDatabase,
    UnresolvedOperation,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum OutputSetup {
    #[default]
    Absent,
    Sentinel,
}

/// Drive the bounded local Library MVP adapter through actual package APIs.
/// An invalid protocol or unclassified production error stops the process.
/// A runnable signed `resolve-local-library` request and response is in
/// `examples/package_mvp_resolve.rs` (`cargo run -p morphir-mck-adapter --example package_mvp_resolve`).
/// A metadata-only `refresh-local-library` example is in
/// `examples/package_mvp_refresh.rs` (`cargo run -p morphir-mck-adapter --example package_mvp_refresh`).
///
/// ```
/// use morphir_mck_adapter::package_mvp::run;
/// use serde_json::Value;
/// use std::io::Cursor;
///
/// let input = b"{\"id\":1,\"op\":\"capabilities\"}\n{\"id\":2,\"op\":\"exit\"}\n";
/// let mut output = Vec::new();
/// run(Cursor::new(input), &mut output)?;
/// let reply: Value = serde_json::from_slice(output.split(|byte| *byte == b'\n').next().unwrap())?;
/// assert_eq!(reply["contractVersion"], "0.1.0-draft.3");
/// assert_eq!(reply["operations"], serde_json::json!(["restore-local-library", "resolve-local-library", "refresh-local-library", "update-local-library"]));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn run(mut reader: impl BufRead, mut writer: impl Write) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut requests = 0usize;
    while let Some(line) = read_bounded_line(&mut reader, MAX_REQUEST_LINE)? {
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
                "profiles":[PROFILE],"operations":["restore-local-library","resolve-local-library","refresh-local-library","update-local-library"]
            }),
            Request::RestoreLocalLibrary {
                profile,
                environment,
                files,
            } => {
                ensure!(profile == PROFILE, "unsupported package MVP profile");
                let files = admit_files(files)?;
                runtime.block_on(restore(files, environment))?
            }
            Request::ResolveLocalLibrary {
                profile,
                exact_root,
                environment,
                files,
            } => {
                ensure!(profile == PROFILE, "unsupported package MVP profile");
                let files = admit_files(files)?;
                runtime.block_on(resolve(files, &exact_root, environment))?
            }
            Request::RefreshLocalLibrary {
                profile,
                environment,
                files,
            } => {
                ensure!(profile == PROFILE, "unsupported package MVP profile");
                let files = admit_refresh_files(files)?;
                runtime.block_on(refresh::execute(files, environment))?
            }
            Request::UpdateLocalLibrary {
                profile,
                targets,
                environment,
                files,
            } => {
                ensure!(profile == PROFILE, "unsupported package MVP profile");
                let files = admit_update_files(files)?;
                runtime.block_on(update::execute(files, &targets, environment))?
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

fn read_bounded_line(reader: &mut impl BufRead, max_bytes: usize) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    let limit = u64::try_from(max_bytes)?
        .checked_add(1)
        .context("request line limit overflow")?;
    let count = std::io::Read::take(reader, limit).read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    ensure!(bytes.len() <= max_bytes, "package MVP request line limit");
    Ok(Some(String::from_utf8(bytes)?))
}

fn admit_files(files: Vec<WireFile>) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(
        (15..=16).contains(&files.len()),
        "package MVP requires 15 or 16 input files"
    );
    let found = decode_files(files)?;
    require_common_inputs(&found)?;
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
            && bundles.values().all(|files| files.contains("ir.json")
                && files.contains("manifest.json")
                && (files.len() == 2 || (files.len() == 3 && files.contains("undeclared.txt")))),
        "package MVP requires two complete bundles"
    );
    ensure!(
        records == 2 && statements == 2,
        "package MVP requires two records and statements"
    );
    Ok(found)
}

fn admit_refresh_files(files: Vec<WireFile>) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(
        (7..=16).contains(&files.len()),
        "package MVP refresh requires 7 to 16 input files"
    );
    let found = decode_files(files)?;
    require_common_inputs(&found)?;
    Ok(found)
}

fn admit_update_files(files: Vec<WireFile>) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(
        (50..=51).contains(&files.len()),
        "package MVP update requires 50 or 51 input files"
    );
    let found = decode_files(files)?;
    require_common_inputs(&found)?;
    for name in [
        "registry/metadata/2.snapshot.json",
        "registry/metadata/2.targets.json",
        "registry/metadata/2.timestamp.json",
    ] {
        ensure!(
            found.contains_key(name),
            "missing package MVP update input: {name}"
        );
    }
    Ok(found)
}

fn decode_files(files: Vec<WireFile>) -> Result<BTreeMap<String, Vec<u8>>> {
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
    Ok(found)
}

fn require_common_inputs(found: &BTreeMap<String, Vec<u8>>) -> Result<()> {
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
    Ok(())
}

fn allowed_path(path: &str) -> bool {
    if path == EXTRA_BUNDLE_INPUT || path == "initialization-policy.json" {
        return true;
    }
    if matches!(
        path,
        "trust-policy.json"
            | "morphir.lock"
            | "registry/metadata/1.root.json"
            | "registry/metadata/1.snapshot.json"
            | "registry/metadata/1.targets.json"
            | "registry/metadata/1.timestamp.json"
            | "registry/metadata/timestamp.json"
            | "registry/metadata/2.snapshot.json"
            | "registry/metadata/2.targets.json"
            | "registry/metadata/2.timestamp.json"
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

async fn restore(
    files: BTreeMap<String, Vec<u8>>,
    environment: Environment,
) -> Result<serde_json::Value> {
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
    let initialization_policy = files.get("initialization-policy.json").unwrap_or(policy);
    let bootstrap = files
        .get("registry/metadata/1.root.json")
        .context("missing root")?;
    let state = root.join("trust");
    let output = root.join("output");
    if environment.trust_state != TrustState::Uninitialized {
        mvp::initialize(mvp::InitializeRequest {
            policy: initialization_policy,
            root: bootstrap,
            state: &state,
        })?;
    }
    match environment.trust_state {
        TrustState::MissingDatabase => fs::remove_file(state.join("trust.sqlite"))?,
        TrustState::CorruptDatabase => {
            fs::write(state.join("trust.sqlite"), b"not a SQLite database\n")?
        }
        TrustState::UnresolvedOperation => {
            fs::write(state.join("operation"), b"unresolved prior operation\n")?
        }
        TrustState::Initialized | TrustState::Uninitialized => {}
    }
    if environment.output == OutputSetup::Sentinel {
        fs::create_dir_all(&output)?;
        fs::write(output.join("sentinel.txt"), b"unrelated consumer content\n")?;
    }
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
        Err(error) => {
            let (category, reason) = classify_refusal(&error)
                .ok_or_else(|| anyhow::anyhow!("package MVP restore failed: {error:#}"))?;
            let (output_state, output_files) = if environment.output == OutputSetup::Sentinel {
                let present = inventory(&output)?;
                ensure!(
                    present.len() == 1 && present.contains_key("sentinel.txt"),
                    "refused restore changed occupied output"
                );
                ensure!(
                    fs::read(output.join("sentinel.txt"))? == b"unrelated consumer content\n",
                    "refused restore changed sentinel bytes"
                );
                (
                    "preserved-sentinel",
                    vec![
                        json!({"path":"sentinel.txt","sha256":Digest::of_bytes(b"unrelated consumer content\n").to_string()}),
                    ],
                )
            } else {
                ensure!(!output.exists(), "refused restore published output");
                ("absent", Vec::new())
            };
            Ok(
                json!({"outcome":"refused","category":category,"reason":reason,"output":output_state,"outputFiles":output_files,"lockUnchanged":lock_unchanged,"registryUnchanged":registry_unchanged}),
            )
        }
    }
}

async fn resolve(
    files: BTreeMap<String, Vec<u8>>,
    exact_root: &str,
    environment: Environment,
) -> Result<serde_json::Value> {
    let (package_path, version) = exact_root
        .rsplit_once('@')
        .context("exact root must contain a version")?;
    let release = ReleaseId::new(
        PackagePath::parse(package_path)?,
        StableVersion::parse(version)?,
    );
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
    let initialization_policy = files.get("initialization-policy.json").unwrap_or(policy);
    let bootstrap = files
        .get("registry/metadata/1.root.json")
        .context("missing root")?;
    let state = root.join("trust");
    let output = root.join("resolved.lock");
    if environment.trust_state != TrustState::Uninitialized {
        mvp::initialize(mvp::InitializeRequest {
            policy: initialization_policy,
            root: bootstrap,
            state: &state,
        })?;
    }
    match environment.trust_state {
        TrustState::MissingDatabase => fs::remove_file(state.join("trust.sqlite"))?,
        TrustState::CorruptDatabase => {
            fs::write(state.join("trust.sqlite"), b"not a SQLite database\n")?
        }
        TrustState::UnresolvedOperation => {
            fs::write(state.join("operation"), b"unresolved prior operation\n")?
        }
        TrustState::Initialized | TrustState::Uninitialized => {}
    }
    if environment.output == OutputSetup::Sentinel {
        fs::write(&output, b"unrelated consumer content\n")?;
    }
    let result = mvp::resolve(mvp::ResolveRequest {
        policy,
        root: release,
        registry: &registry,
        state: &state,
        output: &output,
    })
    .await;
    let lock_unchanged = fs::read(&lock)? == before_lock;
    let registry_unchanged = inventory(&registry)? == before_registry;
    ensure!(
        lock_unchanged && registry_unchanged,
        "resolve changed package MVP inputs"
    );
    match result {
        Ok(_) => {
            let bytes = fs::read(&output).context("resolve did not publish a lock")?;
            Ok(json!({
                "outcome":"resolved", "output":"present",
                "outputFiles":[{"path":"morphir.lock","sha256":Digest::of_bytes(&bytes).to_string()}],
                "lockUnchanged":lock_unchanged,"registryUnchanged":registry_unchanged
            }))
        }
        Err(error) => {
            let (category, reason) = classify_refusal(&error)
                .ok_or_else(|| anyhow::anyhow!("package MVP resolve failed: {error:#}"))?;
            let (output_state, output_files) = if environment.output == OutputSetup::Sentinel {
                ensure!(
                    fs::read(&output)? == b"unrelated consumer content\n",
                    "refused resolve changed occupied output"
                );
                (
                    "preserved-sentinel",
                    vec![
                        json!({"path":"morphir.lock","sha256":Digest::of_bytes(b"unrelated consumer content\n").to_string()}),
                    ],
                )
            } else {
                ensure!(!output.exists(), "refused resolve published output");
                ("absent", Vec::new())
            };
            Ok(json!({
                "outcome":"refused","category":category,"reason":reason,
                "output":output_state,"outputFiles":output_files,
                "lockUnchanged":lock_unchanged,"registryUnchanged":registry_unchanged
            }))
        }
    }
}

fn classify_refusal(error: &mvp::Error) -> Option<(&'static str, &'static str)> {
    match error {
        e if timestamp_signature_failure(e) => {
            Some(("metadata-authentication", "timestamp-signature-threshold"))
        }
        mvp::Error::Refused("trust state is uninitialized or missing") => {
            Some(("trust-state", "uninitialized"))
        }
        mvp::Error::Refused("trust state database is missing") => {
            Some(("trust-state", "missing-established-database"))
        }
        mvp::Error::Refused("unresolved prior operation; manual intervention required") => {
            Some(("trust-state", "unresolved-operation"))
        }
        mvp::Error::Refused("destination already exists") => {
            Some(("output-conflict", "destination-exists"))
        }
        mvp::Error::Refused("content digest mismatch") => {
            Some(("package-integrity", "content-digest-mismatch"))
        }
        mvp::Error::Refused("bundle inventory differs from manifest") => {
            Some(("package-integrity", "bundle-inventory-mismatch"))
        }
        mvp::Error::Refused("published root is unavailable for new selection") => {
            Some(("invalid-input", "published-root-unavailable"))
        }
        mvp::Error::Refused(
            "historical evidence unsupported by MVP; refresh lock metadata pins",
        ) => Some(("unsupported-policy", "historical-evidence-unsupported")),
        mvp::Error::Refused("MVP requires fresh-metadata policy") => {
            Some(("unsupported-policy", "historical-authorization-unsupported"))
        }
        mvp::Error::Document(diagnostic)
            if diagnostic.code == Code::SignatureInvalid
                && diagnostic.phase == Phase::Authorization =>
        {
            Some(("publisher-authorization", "publisher-signature-invalid"))
        }
        mvp::Error::Document(diagnostic)
            if diagnostic.code == Code::UnsafePath && diagnostic.phase == Phase::Shape =>
        {
            Some(("invalid-input", "unsafe-acquisition-path"))
        }
        mvp::Error::Metadata(tuf::MetadataLoadError::Update(source))
            if matches!(
                source.as_ref(),
                error::Error::ExpiredMetadata {
                    role: schema::RoleType::Timestamp,
                    ..
                }
            ) =>
        {
            Some(("metadata-authentication", "timestamp-expired"))
        }
        mvp::Error::State(source)
            if source.sqlite_error_code() == Some(rusqlite::ErrorCode::NotADatabase) =>
        {
            Some(("trust-state", "corrupt-established-database"))
        }
        _ => None,
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

#[cfg(test)]
mod bounded_input_tests {
    use super::read_bounded_line;
    use std::io::Cursor;

    #[test]
    fn rejects_an_oversized_line_before_parsing_and_accepts_the_next_line() {
        let mut input = Cursor::new(b"123456789\n{}\n");
        assert!(read_bounded_line(&mut input, 8).is_err());

        let mut input = Cursor::new(b"12345678\n{}\n");
        assert_eq!(
            read_bounded_line(&mut input, 9).unwrap(),
            Some("12345678\n".into())
        );
        assert_eq!(
            read_bounded_line(&mut input, 9).unwrap(),
            Some("{}\n".into())
        );
        assert_eq!(read_bounded_line(&mut input, 9).unwrap(), None);
    }
}
