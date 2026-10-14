//! Emit explicit test-only signing artifacts and projection interoperability files.
//! No private trust database is exported; the known signing seeds are test keys.
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
use std::{fs, path::Path};

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
    let destination = std::env::args_os()
        .nth(1)
        .ok_or("supply a new output directory for TEST-ONLY fixtures")?;
    let destination = Path::new(&destination);
    fs::create_dir(destination)?;
    let provider = mothers::a_signed_naming_provider().await;
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
    fs::create_dir(destination.join("contexts"))?;
    fs::write(destination.join("contexts/schema.jsonld"), mothers::CONTEXT)?;
    copy_tree(
        &provider.directory.path().join("registry"),
        &destination.join("registry"),
    )?;
    fs::write(
        destination.join("README.md"),
        "# Test-only signed naming provider\n\nKnown test keys sign this local fixture. Never install them as production trust defaults.\nNative IR: morphir/naming 4.1.0. Publisher release: example.com/morphir/naming@1.0.0.\nThe native payload revision is its raw SHA-256, separate from the derived core view.\n",
    )?;
    Ok(())
}
