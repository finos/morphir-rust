use super::*;
use morphir_package::resolution::ResolutionDiagnostic;

pub(super) async fn execute(
    files: BTreeMap<String, Vec<u8>>,
    targets: &[String],
    environment: Environment,
) -> Result<serde_json::Value> {
    let targets = targets
        .iter()
        .map(|target| {
            if let Some((path, version)) = target.rsplit_once('@') {
                Ok(UpdateTarget::Exact {
                    package_path: PackagePath::parse(path)?,
                    version: StableVersion::parse(version)?,
                })
            } else {
                Ok(UpdateTarget::Eligible {
                    package_path: PackagePath::parse(target)?,
                })
            }
        })
        .collect::<Result<Vec<_>>>();
    if targets.is_err() {
        let (output, output_files) = refusal_output(environment.output);
        return Ok(json!({
            "outcome":"refused", "category":"invalid-input", "reason":"invalid-update-targets",
            "output":output, "outputFiles":output_files,
            "lockUnchanged":true, "registryUnchanged":true
        }));
    }
    let targets = targets?;
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
    let output = root.join("updated.lock");
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
    let result = mvp::update(mvp::UpdateRequest {
        policy,
        lock: &before_lock,
        targets: &targets,
        registry: &registry,
        state: &state,
        output: &output,
    })
    .await;
    let lock_unchanged = fs::read(&lock)? == before_lock;
    let registry_unchanged = inventory(&registry)? == before_registry;
    ensure!(
        lock_unchanged && registry_unchanged,
        "update changed package MVP inputs"
    );
    match result {
        Ok(_) => {
            let bytes = fs::read(&output).context("update did not publish a lock")?;
            Ok(json!({
                "outcome":"updated", "output":"present",
                "outputFiles":[{"path":"morphir.lock","sha256":Digest::of_bytes(&bytes).to_string()}],
                "lockUnchanged":lock_unchanged,"registryUnchanged":registry_unchanged
            }))
        }
        Err(error) => {
            let (category, reason) = classify_update_refusal(&error)
                .ok_or_else(|| anyhow::anyhow!("package MVP update failed: {error:#}"))?;
            let (output_state, output_files) = if environment.output == OutputSetup::Sentinel {
                ensure!(
                    fs::read(&output)? == b"unrelated consumer content\n",
                    "refused update changed occupied output"
                );
                (
                    "preserved-sentinel",
                    vec![
                        json!({"path":"morphir.lock","sha256":Digest::of_bytes(b"unrelated consumer content\n").to_string()}),
                    ],
                )
            } else {
                ensure!(!output.exists(), "refused update published output");
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

fn refusal_output(output: OutputSetup) -> (&'static str, Vec<serde_json::Value>) {
    if output == OutputSetup::Sentinel {
        (
            "preserved-sentinel",
            vec![
                json!({"path":"morphir.lock","sha256":Digest::of_bytes(b"unrelated consumer content\n").to_string()}),
            ],
        )
    } else {
        ("absent", Vec::new())
    }
}

fn classify_update_refusal(error: &mvp::Error) -> Option<(&'static str, &'static str)> {
    match error {
        mvp::Error::ResolutionRejected(diagnostic) => match diagnostic.as_ref() {
            ResolutionDiagnostic::InvalidInput { .. } => {
                Some(("invalid-input", "invalid-update-targets"))
            }
            ResolutionDiagnostic::InvalidLock { .. } => Some(("invalid-input", "invalid-old-lock")),
            ResolutionDiagnostic::UpdateScopeConflict { .. } => {
                Some(("resolution", "update-scope-conflict"))
            }
            ResolutionDiagnostic::UnsatisfiableRequirements { .. } => {
                Some(("resolution", "unsatisfiable-requirements"))
            }
            _ => None,
        },
        mvp::Error::Refused(
            "revocation transition unsupported by MVP; manual intervention required",
        ) => Some(("unsupported-policy", "revocation-transition-unsupported")),
        mvp::Error::Refused("old record acquisition mismatch") => {
            Some(("invalid-input", "old-record-acquisition-mismatch"))
        }
        mvp::Error::Document(diagnostic)
            if diagnostic.code == Code::InvalidInput && diagnostic.phase == Phase::Structure =>
        {
            Some(("invalid-input", "invalid-old-lock"))
        }
        _ => classify_refusal(error),
    }
}
