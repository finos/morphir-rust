use super::*;

pub(super) async fn execute(
    files: BTreeMap<String, Vec<u8>>,
    environment: Environment,
) -> Result<serde_json::Value> {
    ensure!(
        environment.output == OutputSetup::Absent,
        "metadata-only refresh requires absent consumer output"
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
    let result = mvp::refresh(mvp::RefreshRequest {
        policy,
        registry: &registry,
        state: &state,
    })
    .await;
    let lock_unchanged = fs::read(&lock)? == before_lock;
    let registry_unchanged = inventory(&registry)? == before_registry;
    ensure!(
        lock_unchanged && registry_unchanged,
        "refresh changed package MVP inputs"
    );
    match result {
        Ok(receipt) => Ok(json!({
            "outcome":"refreshed","receipt":receipt,
            "output":"absent","outputFiles":[],
            "lockUnchanged":lock_unchanged,"registryUnchanged":registry_unchanged
        })),
        Err(error) => {
            let (category, reason) = classify_refusal(&error)
                .ok_or_else(|| anyhow::anyhow!("package MVP refresh failed: {error:#}"))?;
            Ok(json!({
                "outcome":"refused","category":category,"reason":reason,
                "output":"absent","outputFiles":[],
                "lockUnchanged":lock_unchanged,"registryUnchanged":registry_unchanged
            }))
        }
    }
}
