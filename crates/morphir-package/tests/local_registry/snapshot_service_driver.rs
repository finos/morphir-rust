use super::mothers;
use ion_rs::{Blob, Element, Struct, v1_0::Binary};
use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

/// Exercises the actual executable and its filesystem effects.
pub struct TestDriver {
    provider: mothers::SignedProvider,
    result: Option<Output>,
    next: usize,
}
impl std::fmt::Debug for TestDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestDriver")
            .field("next", &self.next)
            .finish_non_exhaustive()
    }
}
impl TestDriver {
    pub async fn provisioned() -> Self {
        Self {
            provider: mothers::a_signed_naming_provider().await,
            result: None,
            next: 0,
        }
    }
    fn execute(&mut self, bytes: &[u8]) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_morphir-provider-snapshot"))
            .current_dir(self.provider.directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        self.result = Some(child.wait_with_output().unwrap());
    }
    pub fn probe(&mut self) {
        let before = std::fs::read_dir(self.provider.directory.path())
            .unwrap()
            .count();
        let bytes = Element::from(
            Struct::builder()
                .with_field("invocation", "bdd-operation")
                .with_field("projectScope", "a".repeat(64))
                .build(),
        )
        .with_annotations(["morphir_provider_capabilities_request_v1"])
        .encode_as(Binary)
        .unwrap();
        self.execute(&bytes);
        assert_eq!(
            before,
            std::fs::read_dir(self.provider.directory.path())
                .unwrap()
                .count()
        );
    }
    pub fn restore(&mut self, release: &str) {
        self.next += 1;
        let root = self.provider.directory.path();
        let bytes = Element::from(
            Struct::builder()
                .with_field("invocation", "bdd-operation")
                .with_field("projectScope", "a".repeat(64))
                .with_field("release", release)
                .with_field("assurance", "portable")
                .with_field("registry", root.join("registry").to_str().unwrap())
                .with_field("state", root.join("trust").to_str().unwrap())
                .with_field(
                    "output",
                    root.join(format!("service-output-{}", self.next))
                        .to_str()
                        .unwrap(),
                )
                .with_field("policy", Blob::from(self.provider.policy.clone()))
                .with_field("lock", Blob::from(self.provider.lock.clone()))
                .build(),
        )
        .with_annotations(["morphir_provider_snapshot_request_v1"])
        .encode_as(Binary)
        .unwrap();
        self.execute(&bytes);
    }
    pub fn corrupt(&self) {
        std::fs::write(
            self.provider
                .directory
                .path()
                .join("registry/metadata/timestamp.json"),
            b"{}",
        )
        .unwrap();
    }
    pub fn assert_capabilities(&self) {
        let output = self.result.as_ref().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let response = Element::read_one(&output.stdout).unwrap();
        assert_eq!(
            response
                .as_struct()
                .unwrap()
                .get("service")
                .unwrap()
                .as_string(),
            Some(morphir_package::provider_snapshot::service::SERVICE_ID)
        );
    }
    pub fn assert_snapshot(&self) {
        let output = self.result.as_ref().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert!(output.stdout.starts_with(&[0xe0, 1, 0, 0xea]));
        let response = Element::read_one(&output.stdout).unwrap();
        let root = response
            .as_struct()
            .unwrap()
            .get("libraries")
            .unwrap()
            .as_list()
            .unwrap()
            .elements()
            .next()
            .unwrap()
            .as_struct()
            .unwrap();
        assert_eq!(
            root.get("native").unwrap().as_blob(),
            Some(self.provider.native.as_slice())
        );
        assert_eq!(root.get("contexts").unwrap().as_list().unwrap().len(), 1);
    }
    pub fn assert_refused(&self, code: &str) {
        let output = self.result.as_ref().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, format!("{code}\n").as_bytes());
        assert!(
            !self
                .provider
                .directory
                .path()
                .join(format!("service-output-{}", self.next))
                .exists()
        );
    }
}
