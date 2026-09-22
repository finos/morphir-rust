use super::mothers;
use morphir_package::local_registry::mvp::{self, RefreshReport, RefreshRequest};
use std::fs;

/// Domain operations for metadata-only refresh acceptance scenarios.
#[derive(Debug)]
pub struct TestDriver {
    directory: tempfile::TempDir,
    policy: Vec<u8>,
    observation: Option<Result<RefreshReport, mvp::Error>>,
    previous: Option<serde_json::Value>,
}
impl TestDriver {
    pub fn provisioned(revoked: bool) -> Self {
        let (directory, policy) = mothers::registry_with_targets(|targets, _| {
            if revoked {
                targets["records/eligibility-1.2.0.json"]["custom"]["morphir"]["status"] =
                    serde_json::json!("revoked");
            }
        });
        fs::write(
            directory.path().join("morphir.lock"),
            b"existing lock bytes\n",
        )
        .unwrap();
        Self {
            directory,
            policy,
            observation: None,
            previous: None,
        }
    }
    pub fn remove_packages(&self) {
        for directory in ["registry/targets", "registry/bundles"] {
            fs::remove_dir_all(self.directory.path().join(directory)).unwrap();
        }
    }
    pub fn corrupt_child(&self) {
        fs::write(
            self.directory
                .path()
                .join("registry/metadata/1.targets.json"),
            b"corrupt",
        )
        .unwrap();
    }
    pub async fn refresh(&mut self) {
        self.previous = self
            .observation
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|r| serde_json::to_value(r).unwrap());
        self.observation = Some(
            mvp::refresh(RefreshRequest {
                policy: &self.policy,
                registry: &self.directory.path().join("registry"),
                state: &self.directory.path().join("trust"),
            })
            .await,
        );
    }
    pub fn assert_observation_only(&self) {
        let report = self.observation.as_ref().unwrap().as_ref().unwrap();
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert_eq!(value["registry"], "local");
        assert!(
            value["timestampDigest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert_eq!(
            fs::read(self.directory.path().join("morphir.lock")).unwrap(),
            b"existing lock bytes\n"
        );
        assert_eq!(fs::read_dir(self.directory.path()).unwrap().count(), 3);
        assert!(!self.directory.path().join("trust/operation").exists());
    }
    pub fn assert_same_digests(&self) {
        assert_eq!(
            self.previous.as_ref().unwrap(),
            &serde_json::to_value(self.observation.as_ref().unwrap().as_ref().unwrap()).unwrap()
        );
    }
    pub fn assert_refused(&self) {
        assert!(self.observation.as_ref().unwrap().is_err());
        assert!(self.directory.path().join("trust/operation").exists());
    }
    pub fn assert_revocation(&self) {
        assert!(
            self.observation
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("revocation")
        );
        self.assert_refused();
    }
    pub async fn assert_restart_refused(&mut self) {
        self.refresh().await;
        assert!(
            self.observation
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("unresolved prior operation")
        );
    }
}
