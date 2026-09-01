//! Shared install/activate fixtures for installed-WebAssembly-extension
//! lifecycle conformance tests.
//!
//! Each backend extension gets its own `tests/installed_wasm_extension_*.rs`
//! integration-test binary so the two extensions' pipelines stay isolated:
//! running one extension's `cargo test --test <binary> -- --ignored` never
//! requires the other extension's release `.wasm` guest to exist. This
//! module holds only the parts that do not depend on which extension is
//! under test — install, activate, and locate the compiled guest.

use morphir_common::home::MorphirHome;
use morphir_distribution::{
    Channel, ExtensionId, ExtensionInstaller, InstalledExtension, LocalIndex, Platform, Selection,
    Sha256Digest,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

#[allow(dead_code)]
pub struct InstalledWasmMother {
    _root: TempDir,
    pub home: MorphirHome,
    index: PathBuf,
    pub workspace: TempDir,
    extension_id: ExtensionId,
}

#[allow(dead_code)]
impl InstalledWasmMother {
    /// Build a controlled local index around one release-built guest and
    /// resolve a fresh `MorphirHome` to install it into.
    pub fn from_path(
        guest_path: impl AsRef<Path>,
        extension_id: &str,
        display_name: &str,
        targets: &[&str],
        ir_versions: &[&str],
    ) -> Self {
        let guest_path = guest_path.as_ref();
        let bytes = fs::read(guest_path).unwrap_or_else(|error| {
            panic!(
                "the release {display_name} WebAssembly guest must exist at {}: {error}",
                guest_path.display()
            )
        });
        let root = tempfile::tempdir().expect("fixture root should be created");
        let index = root.path().join("index");
        let artifact_name = guest_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("guest path should have a UTF-8 file name")
            .to_owned();
        let artifact_path = index.join("artifacts").join(&artifact_name);
        fs::create_dir_all(artifact_path.parent().expect("artifact has a parent"))
            .expect("fixture artifact directory should be created");
        fs::create_dir_all(index.join("extensions"))
            .expect("fixture index directory should be created");
        fs::write(&artifact_path, &bytes).expect("the guest should be copied into the index");

        let extension_id_value =
            ExtensionId::parse(extension_id).expect("fixture ID should be valid");
        let digest = Sha256Digest::of_bytes(&bytes);
        let record = json!({
            "schemaVersion": 2,
            "id": extension_id_value,
            "name": display_name,
            "version": "0.1.0",
            "channels": ["stable"],
            "mepVersions": ["0.1"],
            "capabilities": ["backend"],
            "backend": { "targets": targets, "irVersions": ir_versions },
            "artifacts": [{
                "runtime": "wasm",
                "source": { "kind": "local-file", "path": format!("artifacts/{artifact_name}") },
                "sha256": digest,
                "filename": artifact_name
            }]
        });
        fs::write(
            index.join(format!("extensions/{extension_id}.jsonl")),
            format!("{record}\n"),
        )
        .expect("the controlled index should contain the release record");

        let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None)
            .expect("fixture Morphir home should resolve");
        Self {
            _root: root,
            home,
            index,
            workspace: tempfile::tempdir().expect("fixture workspace should be created"),
            extension_id: extension_id_value,
        }
    }

    pub fn install(&self) -> morphir_distribution::Result<InstalledExtension> {
        let selected = LocalIndex::open(&self.index)?.resolve(
            &self.extension_id,
            Selection::Channel(Channel::Stable),
            &Platform::current(),
        )?;
        ExtensionInstaller::new(&self.home).install(selected)
    }
}

/// The compiled path of one release `wasm32-unknown-unknown` guest artifact.
#[allow(dead_code)]
pub fn wasm_guest_path(artifact_file_name: &str) -> PathBuf {
    cargo_target_directory()
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(artifact_file_name)
}

#[allow(dead_code)]
pub fn cargo_target_directory() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("cargo metadata should start");
    assert!(
        output.status.success(),
        "cargo metadata failed for {}: {}",
        manifest.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata should return JSON");
    let target = metadata["target_directory"]
        .as_str()
        .expect("cargo metadata should report target_directory");
    let target = PathBuf::from(target);
    assert!(
        target.is_absolute(),
        "cargo metadata should report an absolute target_directory: {}",
        target.display()
    );
    target
}
