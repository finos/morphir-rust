use super::fixture;
use ion_rs::{Blob, Element, Struct, v1_0::Binary};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Drives the fixture entry point and then the actual one-shot service.
#[derive(Debug)]
pub struct TestDriver {
    directory: tempfile::TempDir,
    result: Option<Result<(), String>>,
    before: Option<BTreeMap<PathBuf, Vec<u8>>>,
}
impl TestDriver {
    /// Use a private parent and a destination that does not exist.
    pub fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
            result: None,
            before: None,
        }
    }
    fn destination(&self) -> PathBuf {
        self.directory.path().join("exported")
    }
    /// Invoke the fixture entry point with the selected command arguments.
    pub async fn export(&mut self, flag: Option<&str>) {
        let mut args = vec![self.destination().into_os_string()];
        if let Some(flag) = flag {
            args.push(flag.into());
        }
        self.result = Some(fixture::run(args).await.map_err(|e| e.to_string()));
    }
    /// Check the default export without treating saved frames as authority.
    pub fn assert_artifacts_only(&self) {
        assert!(self.result.as_ref().unwrap().is_ok());
        for path in [
            "native.json",
            "core.json",
            "document-metadata.json",
            "snapshot.ionb",
            "capabilities.ionb",
            "registry/metadata/1.root.json",
        ] {
            assert!(self.destination().join(path).is_file(), "{path}");
        }
        assert!(!self.destination().join("test-state").exists());
    }
    /// Restore through the real executable using newly initialized test trust.
    pub fn assert_fresh_authentication(&self) {
        assert!(self.result.as_ref().unwrap().is_ok());
        let destination = self.destination();
        let bytes = Element::from(
            Struct::builder()
                .with_field("invocation", "fixture-qualification")
                .with_field("projectScope", "a".repeat(64))
                .with_field("release", "example.com/morphir/naming@1.0.0")
                .with_field("assurance", "portable")
                .with_field("registry", destination.join("registry").to_str().unwrap())
                .with_field("state", destination.join("test-state").to_str().unwrap())
                .with_field("output", destination.join("restored").to_str().unwrap())
                .with_field(
                    "policy",
                    Blob::from(fs::read(destination.join("trust-policy.json")).unwrap()),
                )
                .with_field(
                    "lock",
                    Blob::from(fs::read(destination.join("morphir.lock")).unwrap()),
                )
                .build(),
        )
        .with_annotations(["morphir_provider_snapshot_request_v1"])
        .encode_as(Binary)
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_morphir-provider-snapshot"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert!(output.stdout.starts_with(&[0xe0, 1, 0, 0xea]));
        let response = Element::read_one(&output.stdout).unwrap();
        let library = response
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
            library.get("native").unwrap().as_blob().unwrap(),
            fs::read(destination.join("native.json")).unwrap()
        );
    }
    /// Retain all exported bytes before a refused repeated invocation.
    pub fn remember_existing(&mut self) {
        assert!(self.result.as_ref().unwrap().is_ok());
        assert!(self.destination().join("test-state").is_dir());
        self.before = Some(snapshot(&self.destination()));
    }
    /// Prove refusal preserved every artifact and trust-state file.
    pub fn assert_preserved(&self) {
        assert!(self.result.as_ref().unwrap().is_err());
        assert!(
            self.before.as_ref().unwrap() == &snapshot(&self.destination()),
            "existing export changed"
        );
    }
    /// Prove invalid arguments refuse before destination creation.
    pub fn assert_no_destination(&self) {
        assert!(self.result.as_ref().unwrap().is_err());
        assert!(!self.destination().exists());
    }
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(snapshot(&path));
        } else {
            result.insert(path.clone(), fs::read(&path).unwrap());
        }
    }
    result
}
