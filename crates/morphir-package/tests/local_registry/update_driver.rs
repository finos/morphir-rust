use morphir_package::{
    local_registry::mvp::{self, InitializeRequest, ResolveReport, RestoreRequest, UpdateRequest},
    resolution::{PackagePath, StableVersion, UpdateTarget},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/local_registry/update-fixture")
}
fn copy(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}
/// Domain driver shared by black-box integration tests and Cucumber scenarios.
#[derive(Debug)]
pub struct TestDriver {
    directory: tempfile::TempDir,
    pub policy: Vec<u8>,
    pub old: Vec<u8>,
    targets: Vec<UpdateTarget>,
    pub result: Option<Result<ResolveReport, mvp::Error>>,
}
impl TestDriver {
    pub fn provisioned(variant: Option<&str>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        copy(
            &fixture().join("registry"),
            &directory.path().join("registry"),
        );
        if let Some(variant) = variant {
            copy(
                &fixture().join("variants").join(variant),
                &directory.path().join("registry/metadata"),
            );
        }
        let policy = fs::read(fixture().join("trust-policy.json")).unwrap();
        let old = fs::read(fixture().join("morphir.lock")).unwrap();
        fs::write(directory.path().join("old.lock"), &old).unwrap();
        mvp::initialize(InitializeRequest {
            policy: &policy,
            root: &fs::read(directory.path().join("registry/metadata/1.root.json")).unwrap(),
            state: &directory.path().join("trust"),
        })
        .unwrap();
        Self {
            directory,
            policy,
            old,
            targets: vec![UpdateTarget::Eligible {
                package_path: PackagePath::parse("example.com/finance/eligibility").unwrap(),
            }],
            result: None,
        }
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }
    pub fn exact(&mut self, version: &str) {
        self.targets = vec![UpdateTarget::Exact {
            package_path: PackagePath::parse("example.com/finance/eligibility").unwrap(),
            version: StableVersion::parse(version).unwrap(),
        }];
    }
    pub fn targets(&mut self, targets: Vec<UpdateTarget>) {
        self.targets = targets;
    }
    pub async fn update(&mut self) {
        self.result = Some(
            mvp::update(UpdateRequest {
                policy: &self.policy,
                lock: &self.old,
                targets: &self.targets,
                registry: &self.path("registry"),
                state: &self.path("trust"),
                output: &self.path("updated.lock"),
            })
            .await,
        );
    }
    pub fn assert_golden(&self, name: &str) {
        self.result.as_ref().unwrap().as_ref().unwrap();
        let actual: serde_json::Value =
            serde_json::from_slice(&fs::read(self.path("updated.lock")).unwrap()).unwrap();
        let expected: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture().join("expected").join(name)).unwrap())
                .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(fs::read(self.path("old.lock")).unwrap(), self.old);
        assert!(!self.path("trust/operation").exists());
    }
    pub fn assert_refused(&self, reason: &str) {
        let error = self.result.as_ref().unwrap().as_ref().unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
        assert!(!self.path("updated.lock").exists());
        assert_eq!(
            fs::read(self.path("old.lock")).unwrap(),
            fs::read(fixture().join("morphir.lock")).unwrap()
        );
    }
    pub async fn restore(&self) {
        let result = mvp::restore(RestoreRequest {
            policy: &self.policy,
            lock: &fs::read(self.path("updated.lock")).unwrap(),
            registry: &self.path("registry"),
            state: &self.path("trust"),
            output: &self.path("restored"),
        })
        .await
        .unwrap();
        assert_eq!(result.packages.len(), 4);
        for package in result.packages {
            assert!(
                self.path("restored")
                    .join(package.directory)
                    .join("ir.json")
                    .is_file()
            );
        }
    }
}
