use super::super::package::{ToolPackageFile, VerifiedToolPackage};
use crate::state_io::{StateWriter, atomic_write_bytes};
use crate::{
    Channel, Platform, RelativeArtifactPath, Selection, Sha256Digest, ToolId, ToolReleaseStatus,
};
use morphir_common::home::MorphirHome;
use semver::Version;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(super) struct FailingCatalogWriter {
    pub(super) catalog_path: PathBuf,
}

impl StateWriter for FailingCatalogWriter {
    fn write(&self, path: &Path, bytes: &[u8]) -> crate::Result<()> {
        atomic_write_bytes(path, bytes)?;
        if path == self.catalog_path {
            return Err(crate::DistributionError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("injected tool catalog failure"),
            });
        }
        Ok(())
    }
}

pub(super) fn package(home: &MorphirHome, version: &str, bytes: &[u8]) -> VerifiedToolPackage {
    package_for(
        home,
        "desktop",
        "Morphir Desktop",
        "desktop.exe",
        version,
        bytes,
    )
}

pub(super) fn package_for(
    home: &MorphirHome,
    id: &str,
    name: &str,
    filename: &str,
    version: &str,
    bytes: &[u8],
) -> VerifiedToolPackage {
    let digest = Sha256Digest::of_bytes(bytes);
    let relative =
        RelativeArtifactPath::parse(format!("store/tools/sha256/{digest}/{filename}")).unwrap();
    let path = home.root().join(relative.as_path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let file = ToolPackageFile {
        path: relative.clone(),
        digest: digest.clone(),
        length: bytes.len() as u64,
        executable: true,
    };
    VerifiedToolPackage {
        selection: Selection::Channel(Channel::Stable),
        tool_id: ToolId::parse(id).unwrap(),
        tool_name: name.to_owned(),
        version: Version::parse(version).unwrap(),
        status: ToolReleaseStatus::Active,
        platform: Platform::new("windows", "x86_64").unwrap(),
        digest,
        length: bytes.len() as u64,
        snapshot_version: 1,
        target_path: RelativeArtifactPath::parse(format!("artifacts/{id}/{version}/{filename}"))
            .unwrap(),
        store_path: relative,
        package_root: None,
        args: vec!["--morphir-home".to_owned()],
        files: vec![file],
    }
}

pub(super) fn raw_download(
    root: &Path,
    version: &str,
    bytes: &[u8],
) -> (
    crate::ResolvedTrustedToolArtifact,
    crate::DownloadedToolArtifact,
) {
    let download_root = root.join(format!("repair-{version}"));
    fs::create_dir_all(&download_root).unwrap();
    let download = download_root.join("desktop.exe");
    fs::write(&download, bytes).unwrap();
    let release: crate::ToolReleaseRecord = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": version,
        "channels": [],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": format!("artifacts/desktop/{version}/desktop.exe"),
            "platform": { "os": "windows", "arch": "x86_64" },
            "archive": { "format": "raw", "entryPoint": "desktop.exe" },
            "launch": {
                "kind": "executable",
                "path": "desktop.exe",
                "args": ["--morphir-home"]
            }
        }]
    }))
    .unwrap();
    (
        crate::ResolvedTrustedToolArtifact::test_fixture(
            release,
            Selection::Exact(Version::parse(version).unwrap()),
            Sha256Digest::of_bytes(bytes),
            bytes.len() as u64,
        ),
        crate::DownloadedToolArtifact::test_fixture(download),
    )
}

pub(super) fn zip_release(entry_point: &str) -> crate::ToolReleaseRecord {
    serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": "artifacts/desktop/1.0.0/desktop.zip",
            "platform": { "os": "windows", "arch": "x86_64" },
            "archive": { "format": "zip", "entryPoint": entry_point },
            "launch": {
                "kind": "executable",
                "path": entry_point,
                "args": ["--morphir-home"]
            }
        }]
    }))
    .unwrap()
}

pub(super) fn tar_gzip_release() -> crate::ToolReleaseRecord {
    serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": "artifacts/desktop/1.0.0/linux-x86_64.tar.gz",
            "platform": { "os": "linux", "arch": "x86_64" },
            "archive": { "format": "tar-gzip", "entryPoint": "morphir-desktop" },
            "launch": {
                "kind": "executable",
                "path": "morphir-desktop",
                "args": []
            }
        }]
    }))
    .unwrap()
}

pub(super) fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = fs::File::create(path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    for (name, bytes) in entries {
        archive
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap();
}

pub(super) fn write_tar_gzip(path: &Path, entries: &[(&str, &[u8])]) {
    let entries = entries
        .iter()
        .map(|(name, bytes)| (*name, *bytes, 0o644))
        .collect::<Vec<_>>();
    write_tar_gzip_with_modes(path, &entries);
}

pub(super) fn write_tar_gzip_with_modes(path: &Path, entries: &[(&str, &[u8], u32)]) {
    let gzip =
        flate2::write::GzEncoder::new(fs::File::create(path).unwrap(), flate2::Compression::none());
    let mut archive = tar::Builder::new(gzip);
    for (name, bytes, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        archive.append_data(&mut header, *name, *bytes).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
}

pub(super) fn write_tar_gzip_link(path: &Path, name: &str, link: &str) {
    let gzip =
        flate2::write::GzEncoder::new(fs::File::create(path).unwrap(), flate2::Compression::none());
    let mut archive = tar::Builder::new(gzip);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_path(name).unwrap();
    header.set_link_name(link).unwrap();
    header.set_cksum();
    archive.append(&header, io::empty()).unwrap();
    archive.into_inner().unwrap().finish().unwrap();
}
