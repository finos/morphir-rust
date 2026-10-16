use ion_rs::{Blob, Element, Struct, v1_0::Binary};
use morphir_package::{local_registry::mvp, provider_snapshot::service::SERVICE_ID};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

/// Selects the packaged production service.
pub const SERVICE_VARIABLE: &str = "MORPHIR_PROVIDER_RELEASE_SERVICE";
/// Selects the packaged test-only fixture exporter.
pub const FIXTURE_VARIABLE: &str = "MORPHIR_PROVIDER_RELEASE_FIXTURE";
/// Optional new directory that receives observed qualification evidence.
pub const EVIDENCE_VARIABLE: &str = "MORPHIR_PROVIDER_RELEASE_EVIDENCE";

const RELEASE: &str = "example.com/morphir/naming@1.0.0";
const ION_VERSION_MARKER: [u8; 4] = [0xe0, 1, 0, 0xea];

/// Drives packaged executables only as child processes in a new directory.
#[derive(Debug)]
pub struct TestDriver {
    service: PathBuf,
    fixture: PathBuf,
    evidence: Option<PathBuf>,
    directory: tempfile::TempDir,
    exported: Option<Output>,
    response: Option<Output>,
}
impl TestDriver {
    /// Read both packaged executable paths. Each must be an absolute file.
    pub fn from_environment() -> Self {
        Self {
            service: executable(SERVICE_VARIABLE),
            fixture: executable(FIXTURE_VARIABLE),
            evidence: std::env::var_os(EVIDENCE_VARIABLE).map(PathBuf::from),
            directory: tempfile::tempdir().unwrap(),
            exported: None,
            response: None,
        }
    }
    fn destination(&self) -> PathBuf {
        self.directory.path().join("exported")
    }
    fn output(&self) -> PathBuf {
        self.directory.path().join("restored")
    }
    fn run_service(&mut self, request: &[u8]) {
        let mut child = Command::new(&self.service)
            .current_dir(self.directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(request).unwrap();
        self.response = Some(child.wait_with_output().unwrap());
    }
    /// Send a capabilities probe with no restore inputs.
    pub fn probe(&mut self) {
        let request = Element::from(
            Struct::builder()
                .with_field("invocation", "release-qualification")
                .with_field("projectScope", "a".repeat(64))
                .build(),
        )
        .with_annotations(["morphir_provider_capabilities_request_v1"])
        .encode_as(Binary)
        .unwrap();
        self.run_service(&request);
        assert_eq!(fs::read_dir(self.directory.path()).unwrap().count(), 0);
    }
    /// Check the protocol identity and record it as release evidence.
    pub fn assert_capabilities(&self) {
        let response = self.successful_response("morphir_provider_capabilities_response_v1");
        let fields = response.as_struct().unwrap();
        let text = |name| fields.get(name).unwrap().as_string().unwrap().to_owned();
        assert_eq!(text("service"), SERVICE_ID);
        assert_eq!(text("profile"), mvp::PROFILE);
        assert_eq!(text("profileVersion"), mvp::PROFILE_VERSION);
        self.record(
            "capabilities.json",
            serde_json::json!({
                "service": text("service"),
                "profile": text("profile"),
                "profileVersion": text("profileVersion"),
                "codec": text("codec"),
                "requestAnnotations": [
                    "morphir_provider_capabilities_request_v1",
                    "morphir_provider_snapshot_request_v1"
                ],
                "responseAnnotations": [
                    "morphir_provider_capabilities_response_v1",
                    "morphir_provider_snapshot_response_v1"
                ]
            }),
        );
    }
    fn export(&mut self, args: &[&str]) {
        let mut command = Command::new(&self.fixture);
        command.arg(self.destination());
        command.args(args.iter().map(OsString::from));
        self.exported = Some(command.output().unwrap());
    }
    /// Export with no trust flag.
    pub fn export_artifacts_only(&mut self) {
        self.export(&[]);
    }
    /// Export with an option the exporter must refuse.
    pub fn export_unknown_option(&mut self) {
        self.export(&["--implicit-trust"]);
    }
    /// Export one signed variant with destination-local fresh test trust.
    pub fn export_variant(&mut self, variant: &str) {
        self.export(&["--provision-test-trust", "--variant", variant]);
        let exported = self.exported.as_ref().unwrap();
        assert!(
            exported.status.success(),
            "{}",
            String::from_utf8_lossy(&exported.stderr)
        );
        assert!(self.destination().join("test-state").is_dir());
    }
    /// Check signed artifacts exist and no trust database was created.
    pub fn assert_artifacts_only(&self) {
        let exported = self.exported.as_ref().unwrap();
        assert!(
            exported.status.success(),
            "{}",
            String::from_utf8_lossy(&exported.stderr)
        );
        for path in [
            "native.json",
            "core.json",
            "document-metadata.json",
            "trust-policy.json",
            "morphir.lock",
            "registry/metadata/1.root.json",
        ] {
            assert!(self.destination().join(path).is_file(), "{path}");
        }
        assert!(!self.destination().join("test-state").exists());
    }
    /// Check refusal happened before the destination existed.
    pub fn assert_no_destination(&self) {
        assert!(!self.exported.as_ref().unwrap().status.success());
        assert!(!self.destination().exists());
    }
    /// Make the exported current timestamp unusable.
    pub fn invalidate_timestamp(&self) {
        fs::write(
            self.destination().join("registry/metadata/timestamp.json"),
            b"{}",
        )
        .unwrap();
    }
    /// Request the locked naming release with the exported inputs.
    pub fn restore(&mut self) {
        let destination = self.destination();
        let request = Element::from(
            Struct::builder()
                .with_field("invocation", "release-qualification")
                .with_field("projectScope", "a".repeat(64))
                .with_field("release", RELEASE)
                .with_field("assurance", "portable")
                .with_field("registry", destination.join("registry").to_str().unwrap())
                .with_field("state", destination.join("test-state").to_str().unwrap())
                .with_field("output", self.output().to_str().unwrap())
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
        self.run_service(&request);
    }
    /// Compare retained bytes with the exported files and check the variant.
    pub fn assert_variant(&self, variant: &str) {
        let response = self.successful_response("morphir_provider_snapshot_response_v1");
        let destination = self.destination();
        let fields = response.as_struct().unwrap();
        assert_eq!(fields.get("release").unwrap().as_string(), Some(RELEASE));
        assert_eq!(
            fields.get("profileVersion").unwrap().as_string(),
            Some(mvp::PROFILE_VERSION)
        );
        let core = fields.get("core").unwrap().as_blob().unwrap();
        assert_eq!(core, fs::read(destination.join("core.json")).unwrap());
        serde_json::from_slice::<serde_json::Value>(core).unwrap();
        assert_eq!(
            fields.get("metadata").unwrap().as_blob().unwrap(),
            fs::read(destination.join("document-metadata.json")).unwrap()
        );
        let library = first_library(&response);
        let native_bytes = library.get("native").unwrap().as_blob().unwrap();
        assert_eq!(
            native_bytes,
            fs::read(destination.join("native.json")).unwrap()
        );
        let contexts: Vec<_> = library
            .get("contexts")
            .unwrap()
            .as_list()
            .unwrap()
            .elements()
            .map(|c| c.as_struct().unwrap())
            .collect();
        for context in &contexts {
            let bytes = context.get("bytes").unwrap().as_blob().unwrap();
            assert_eq!(
                context.get("sha256").unwrap().as_string().unwrap(),
                format!("sha256:{}", hex(&Sha256::digest(bytes)))
            );
        }
        let native: serde_json::Value = serde_json::from_slice(native_bytes).unwrap();
        let declaration = &native["$meta"]["@graph"][0];
        let output_type = &native["distribution"]["Library"]["def"]["modules"]["naming"]["Public"]
            ["values"]["target-names"]["Public"]["ExpressionBody"]["outputType"];
        match variant {
            "compatible" => {
                assert_eq!(contexts.len(), 1);
                assert_eq!(declaration["interpreter"], "target-name-language-ids");
            }
            "wrong-type" => assert_eq!(output_type, "morphir/SDK:basics#bool"),
            "wrong-subject-role" => assert_eq!(declaration["subject-role"], "ValueSpecification"),
            "wrong-object-form" => assert_eq!(declaration["object-form"], "iri"),
            "wrong-interpreter" => assert_eq!(declaration["interpreter"], "descriptive"),
            "ambiguous-contexts" => {
                assert_eq!(contexts.len(), 2);
                assert_eq!(contexts[0].get("bytes"), contexts[1].get("bytes"));
                let mut paths: Vec<_> = contexts
                    .iter()
                    .map(|c| c.get("path").unwrap().as_string().unwrap())
                    .collect();
                paths.sort();
                assert_eq!(paths, ["contexts/alias.jsonld", "contexts/schema.jsonld"]);
            }
            _ => panic!("unknown expected variant"),
        }
        self.record(
            &format!("variant-{variant}.json"),
            serde_json::json!({
                "variant": variant,
                "release": RELEASE,
                "revision": library.get("revision").unwrap().as_string().unwrap(),
                "contexts": contexts.len(),
                "coreSha256": hex(&Sha256::digest(core)),
            }),
        );
    }
    /// Check a fail-closed refusal with no restored output.
    pub fn assert_refused(&self, code: &str) {
        let response = self.response.as_ref().unwrap();
        assert!(!response.status.success());
        assert!(response.stdout.is_empty());
        assert_eq!(response.stderr, format!("{code}\n").as_bytes());
        assert!(!self.output().exists());
    }
    fn successful_response(&self, annotation: &str) -> Element {
        let response = self.response.as_ref().unwrap();
        assert!(
            response.status.success(),
            "{}",
            String::from_utf8_lossy(&response.stderr)
        );
        assert!(response.stderr.is_empty());
        assert!(response.stdout.starts_with(&ION_VERSION_MARKER));
        let element = Element::read_one(&response.stdout).unwrap();
        assert_eq!(
            element.annotations().iter().next().unwrap().text(),
            Some(annotation)
        );
        element
    }
    fn record(&self, name: &str, value: serde_json::Value) {
        if let Some(evidence) = &self.evidence {
            fs::create_dir_all(evidence).unwrap();
            let mut bytes = serde_json::to_vec_pretty(&value).unwrap();
            bytes.push(b'\n');
            fs::write(evidence.join(name), bytes).unwrap();
        }
    }
}
fn executable(variable: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(variable)
            .unwrap_or_else(|| panic!("set {variable} to a packaged executable")),
    );
    assert!(path.is_absolute(), "{variable} must be an absolute path");
    assert!(Path::new(&path).is_file(), "{variable} is not a file");
    path
}
fn first_library(response: &Element) -> &Struct {
    response
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
        .unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
