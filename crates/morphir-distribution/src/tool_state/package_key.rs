//! Deterministic namespaces for isolated tool-package launch contracts.

use crate::{ArchiveFormat, RelativeArtifactPath, Sha256Digest, ToolArtifactRecord};
use std::path::{Path, PathBuf};

pub(super) fn extracted_package_path(
    digest_directory: &Path,
    artifact: &ToolArtifactRecord,
) -> PathBuf {
    package_path(
        digest_directory,
        artifact.archive().format(),
        artifact.launch().path(),
    )
}

pub(super) fn package_path(
    digest_directory: &Path,
    format: ArchiveFormat,
    entry_point: &RelativeArtifactPath,
) -> PathBuf {
    let format = match format {
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::TarGzip => "tar-gzip",
        ArchiveFormat::Appimage => "appimage",
        ArchiveFormat::Raw => "raw",
    };
    let contract = format!(
        "morphir-extracted-package-v1\0{format}\0{}",
        entry_point.as_str()
    );
    let key = Sha256Digest::of_bytes(contract.as_bytes());
    digest_directory
        .join("packages")
        .join(format!("package-{key}"))
}
