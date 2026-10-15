//! Emit explicit test-only signing artifacts and projection interoperability files.
//! By default no private trust database is exported. Known signing seeds are test keys.
//! Add `--provision-test-trust` after the new destination to initialize fresh,
//! destination-local test trust for real service qualification. Production hosts
//! must never call this example or install its test keys as trust defaults.
//! `--variant` selects signed semantic incompatibilities or duplicate context
//! bytes at distinct inventory paths. These are authenticated test artifacts,
//! not declarations that a consumer must admit.
#[path = "../tests/local_registry/snapshot_mothers.rs"]
#[allow(dead_code)]
mod mothers;
#[path = "../tests/local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;

use morphir_package::{
    local_registry::mvp::{self, RestoreRequest},
    provider_snapshot::project_library,
};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
enum TestTrust {
    Omit,
    InitializeFresh,
}

#[derive(Debug)]
struct ExportRequest {
    destination: PathBuf,
    trust: TestTrust,
    variant: mothers::NamingVariant,
}
impl ExportRequest {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, &'static str> {
        let mut args = args.into_iter();
        let destination = args
            .next()
            .ok_or("supply a new output directory for TEST-ONLY fixtures")?;
        if destination.as_encoded_bytes().starts_with(b"--") {
            return Err("supply the new output directory before --provision-test-trust");
        }
        let mut trust = TestTrust::Omit;
        let mut variant = None;
        while let Some(flag) = args.next() {
            if flag == "--provision-test-trust" && trust == TestTrust::Omit {
                trust = TestTrust::InitializeFresh;
            } else if flag == "--variant" && variant.is_none() {
                let value = args.next().ok_or("--variant requires a value")?;
                variant = Some(mothers::NamingVariant::parse(
                    value.to_str().ok_or("non-UTF-8 variant")?,
                )?);
            } else {
                return Err("unknown or repeated fixture option");
            }
        }
        Ok(Self {
            destination: destination.into(),
            trust,
            variant: variant.unwrap_or_default(),
        })
    }
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(std::env::args_os().skip(1)).await
}

/// Export a new caller-selected directory of signed test-only artifacts.
pub(crate) async fn run(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(), Box<dyn std::error::Error>> {
    let export = ExportRequest::parse(args)?;
    let destination = export.destination.as_path();
    fs::create_dir(destination)?;
    let provider = mothers::a_signed_naming_variant(export.variant).await;
    let snapshot = mvp::restore_snapshot(RestoreRequest {
        policy: &provider.policy,
        lock: &provider.lock,
        registry: &provider.directory.path().join("registry"),
        state: &provider.directory.path().join("trust"),
        output: &provider.directory.path().join("out"),
    })
    .await?;
    let library = &snapshot.libraries()[0];
    let projection = project_library(library)?;
    fs::write(destination.join("native.json"), library.payload().bytes())?;
    fs::write(destination.join("core.json"), projection.core_json())?;
    fs::write(
        destination.join("document-metadata.json"),
        projection
            .document_metadata_json()
            .ok_or("missing native metadata")?,
    )?;
    fs::write(destination.join("trust-policy.json"), &provider.policy)?;
    fs::write(destination.join("morphir.lock"), &provider.lock)?;
    copy_tree(
        &provider
            .directory
            .path()
            .join("registry")
            .join(&provider.bundle)
            .join("contexts"),
        &destination.join("contexts"),
    )?;
    copy_tree(
        &provider.directory.path().join("registry"),
        &destination.join("registry"),
    )?;
    let request = ion_rs::Element::from(
        ion_rs::Struct::builder()
            .with_field("invocation", "test-operation")
            .with_field("projectScope", "a".repeat(64))
            .with_field("release", "example.com/morphir/naming@1.0.0")
            .with_field("assurance", "portable")
            .with_field(
                "registry",
                provider.directory.path().join("registry").to_str().unwrap(),
            )
            .with_field(
                "state",
                provider.directory.path().join("trust").to_str().unwrap(),
            )
            .with_field(
                "output",
                provider
                    .directory
                    .path()
                    .join("service-output")
                    .to_str()
                    .unwrap(),
            )
            .with_field("policy", ion_rs::Blob::from(provider.policy.clone()))
            .with_field("lock", ion_rs::Blob::from(provider.lock.clone()))
            .build(),
    )
    .with_annotations(["morphir_provider_snapshot_request_v1"])
    .encode_as(ion_rs::v1_0::Binary)?;
    fs::write(
        destination.join("snapshot.ionb"),
        morphir_package::provider_snapshot::service::serve_request(&request).await?,
    )?;
    let probe = ion_rs::Element::from(
        ion_rs::Struct::builder()
            .with_field("invocation", "test-operation")
            .with_field("projectScope", "a".repeat(64))
            .build(),
    )
    .with_annotations(["morphir_provider_capabilities_request_v1"])
    .encode_as(ion_rs::v1_0::Binary)?;
    fs::write(
        destination.join("capabilities.ionb"),
        morphir_package::provider_snapshot::service::serve_request(&probe).await?,
    )?;
    let trust_note = match export.trust {
        TestTrust::Omit => "No destination trust database was initialized.\n",
        TestTrust::InitializeFresh => {
            mvp::initialize(mvp::InitializeRequest {
                policy: &provider.policy,
                root: &fs::read(destination.join("registry/metadata/1.root.json"))?,
                state: &destination.join("test-state"),
            })?;
            "Fresh test-state was initialized by explicit --provision-test-trust. Never use this test trust in production.\n"
        }
    };
    fs::write(
        destination.join("README.md"),
        format!(
            "# Test-only signed naming provider\n\nKnown test keys sign this local fixture. Never install them as production trust defaults.\nNative IR: morphir/naming 4.1.0. Publisher release: example.com/morphir/naming@1.0.0.\nThe native payload revision is its raw SHA-256, separate from the derived core view.\n{trust_note}"
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixture_options_are_explicit_and_validated_before_export() {
        use super::{ExportRequest, TestTrust};
        use std::{ffi::OsString, path::PathBuf};
        let artifact = ExportRequest::parse([OsString::from("new-output")]).unwrap();
        assert_eq!(artifact.trust, TestTrust::Omit);
        let explicit = ExportRequest::parse([
            OsString::from("new-output"),
            OsString::from("--provision-test-trust"),
        ])
        .unwrap();
        assert_eq!(explicit.trust, TestTrust::InitializeFresh);
        assert_eq!(explicit.destination, PathBuf::from("new-output"));
        for args in [
            vec![
                "new-output",
                "--variant",
                "wrong-type",
                "--provision-test-trust",
            ],
            vec![
                "new-output",
                "--provision-test-trust",
                "--variant",
                "wrong-type",
            ],
        ] {
            let parsed = ExportRequest::parse(args.into_iter().map(OsString::from)).unwrap();
            assert_eq!(parsed.variant, super::mothers::NamingVariant::WrongType);
            assert_eq!(parsed.trust, TestTrust::InitializeFresh);
        }
        for args in [
            vec![],
            vec!["--provision-test-trust"],
            vec!["--implicit-trust", "new-output"],
            vec!["new-output", "--automatic-trust"],
            vec![
                "new-output",
                "--provision-test-trust",
                "--provision-test-trust",
            ],
            vec!["new-output", "--provision-test-trust", "extra"],
            vec!["new-output", "--variant"],
            vec!["new-output", "--variant", "unknown"],
            vec![
                "new-output",
                "--variant",
                "compatible",
                "--variant",
                "wrong-type",
            ],
        ] {
            assert!(ExportRequest::parse(args.into_iter().map(OsString::from)).is_err());
        }
    }
}
