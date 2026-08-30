//! Deterministic namespaces for extracted archive launch contracts.

use crate::{ArchiveFormat, Sha256Digest, ToolArtifactRecord};
use std::path::{Path, PathBuf};

pub(super) fn extracted_package_path(
    digest_directory: &Path,
    artifact: &ToolArtifactRecord,
) -> PathBuf {
    let format = match artifact.archive().format() {
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::TarGzip => "tar-gzip",
        ArchiveFormat::Appimage => "appimage",
        ArchiveFormat::Raw => "raw",
    };
    let contract = format!(
        "morphir-extracted-package-v1\0{format}\0{}",
        artifact.launch().path().as_str()
    );
    let key = Sha256Digest::of_bytes(contract.as_bytes());
    digest_directory.join(format!("package-{key}"))
}
