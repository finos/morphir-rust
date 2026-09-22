use super::mothers::{registry_with_targets, root};
use morphir_package::local_registry::{
    decode_library_lock,
    mvp::{self, ResolveReport, ResolveRequest, RestoreReport, RestoreRequest},
};
use std::{fs, path::PathBuf};

/// Domain operations and observations for the executable resolve scenarios.
#[derive(Debug)]
pub struct TestDriver {
    directory: tempfile::TempDir,
    policy: Vec<u8>,
    resolution: Option<Result<ResolveReport, mvp::Error>>,
    restoration: Option<RestoreReport>,
}
impl TestDriver {
    pub fn provisioned_registry() -> Self {
        let (directory, policy) = registry_with_targets(|_, _| {});
        Self {
            directory,
            policy,
            resolution: None,
            restoration: None,
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }
    pub fn occupy_destination(&self) {
        fs::write(self.path("resolved.lock"), b"existing lock bytes\n").unwrap();
    }
    pub fn corrupt_provider(&self) {
        fs::write(self.path("registry/bundles/5922bc8860f6cd008b9cda341be7f3a776ea332e63261392e94c17e19a647886/ir.json"), b"corrupt provider").unwrap();
    }
    pub async fn resolve(&mut self) {
        self.resolution = Some(
            mvp::resolve(ResolveRequest {
                policy: &self.policy,
                root: root(),
                registry: &self.path("registry"),
                state: &self.path("trust"),
                output: &self.path("resolved.lock"),
            })
            .await,
        );
    }
    pub fn assert_complete_lock(&self) {
        let report = self.resolution.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(report.graph.nodes().len(), 2);
        let bytes = fs::read(self.path("resolved.lock")).unwrap();
        let lock = decode_library_lock(&bytes).unwrap();
        assert_eq!(lock.graph(), &report.graph);
        assert_eq!(lock.acquisitions().len(), 2);
        assert_eq!(lock.evidence().len(), 6);
        assert_eq!(report.packages.len(), 2);
        assert!(!self.path("trust/operation").exists());
    }
    pub async fn restore_generated_lock(&mut self) {
        self.restoration = Some(
            mvp::restore(RestoreRequest {
                policy: &self.policy,
                lock: &fs::read(self.path("resolved.lock")).unwrap(),
                registry: &self.path("registry"),
                state: &self.path("trust"),
                output: &self.path("restored"),
            })
            .await
            .unwrap(),
        );
    }
    pub fn assert_materialized(&self) {
        let report = self.restoration.as_ref().unwrap();
        assert_eq!(report.packages.len(), 2);
        for package in &report.packages {
            let directory = self.path("restored").join(&package.directory);
            assert!(directory.join("manifest.json").is_file());
            assert!(directory.join("ir.json").is_file());
        }
    }
    pub async fn resolve_again(&self) {
        self.resolution.as_ref().unwrap().as_ref().unwrap();
        mvp::resolve(ResolveRequest {
            policy: &self.policy,
            root: root(),
            registry: &self.path("registry"),
            state: &self.path("trust"),
            output: &self.path("repeated.lock"),
        })
        .await
        .unwrap();
    }
    pub fn assert_deterministic(&self) {
        assert_eq!(
            fs::read(self.path("resolved.lock")).unwrap(),
            fs::read(self.path("repeated.lock")).unwrap()
        );
    }
    pub fn assert_occupied_unchanged(&self) {
        let error = self.resolution.as_ref().unwrap().as_ref().unwrap_err();
        assert!(error.to_string().contains("destination already exists"));
        assert_eq!(
            fs::read(self.path("resolved.lock")).unwrap(),
            b"existing lock bytes\n"
        );
    }
    pub fn assert_no_operation(&self) {
        assert!(!self.path("trust/operation").exists());
    }
    pub fn assert_no_lock(&self) {
        let error = self.resolution.as_ref().unwrap().as_ref().unwrap_err();
        assert!(
            error.to_string().contains("content digest mismatch"),
            "{error}"
        );
        assert!(!self.path("resolved.lock").exists());
        assert!(self.path("trust/operation").exists());
    }
    pub async fn assert_restart_refused(&mut self) {
        self.resolve().await;
        let error = self.resolution.as_ref().unwrap().as_ref().unwrap_err();
        assert!(
            error.to_string().contains("unresolved prior operation"),
            "{error}"
        );
        assert!(!self.path("resolved.lock").exists());
    }
}
