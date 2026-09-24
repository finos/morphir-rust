//! Installed process and Wasm fixtures shared by the daemon and native host tests.

use morphir_common::home::MorphirHome;
use morphir_distribution::{
    Channel, ExtensionId, ExtensionInstaller, LocalIndex, Platform, Selection, Sha256Digest,
    VerifiedExtensionArtifact, activate_installed,
};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

pub struct RuntimeArtifact {
    _root: TempDir,
    pub artifact: VerifiedExtensionArtifact,
    pub installed_path: PathBuf,
    #[cfg(unix)]
    pub staging_directory: PathBuf,
    pub working_directory: PathBuf,
}

#[derive(Clone, Copy)]
enum MetadataShape {
    #[cfg(unix)]
    Backend,
    FrontendBackend,
    FrontendWorkspace,
    FrontendClaims(InstalledFrontend),
}

#[derive(Clone, Copy)]
pub enum InstalledFrontend {
    Legacy,
    ClaimsWithMultiDocument,
    ClaimsWithoutMultiDocument,
}

impl InstalledFrontend {
    fn metadata(self) -> MetadataShape {
        match self {
            Self::Legacy => MetadataShape::FrontendBackend,
            Self::ClaimsWithMultiDocument | Self::ClaimsWithoutMultiDocument => {
                MetadataShape::FrontendClaims(self)
            }
        }
    }
}

struct ArtifactSpec<'a> {
    id: &'a str,
    name: &'a str,
    runtime: &'a str,
    filename: &'a str,
    bytes: &'a [u8],
    args: &'a [String],
    metadata: MetadataShape,
}

#[cfg(unix)]
pub fn process() -> (RuntimeArtifact, PathBuf, Vec<String>) {
    let root = tempfile::tempdir().unwrap();
    let capture = root.path().join("observed-launch.txt");
    let args = vec![
        capture.to_string_lossy().into_owned(),
        "first argument".to_owned(),
        "--flag=two".to_owned(),
        "café-東京".to_owned(),
        String::new(),
    ];
    let program = b"#!/bin/sh\nprintf '%s\\n' \"$PWD\" > \"$1\"\nprintf '%s\\n' \"$#\" >> \"$1\"\nfor argument do printf '<%s>\\n' \"$argument\" >> \"$1\"; done\nwhile IFS= read -r line; do :; done\n";
    let artifact = install(
        root,
        ArtifactSpec {
            id: "morphir-process",
            name: "Morphir Process",
            runtime: "process",
            filename: "morphir-process",
            bytes: program,
            args: &args,
            metadata: MetadataShape::FrontendBackend,
        },
    );
    (artifact, capture, args)
}

#[cfg(unix)]
pub fn backend_process() -> RuntimeArtifact {
    install(
        tempfile::tempdir().unwrap(),
        ArtifactSpec {
            id: "morphir-backend",
            name: "Morphir Backend",
            runtime: "process",
            filename: "morphir-backend",
            bytes: b"#!/bin/sh\nwhile IFS= read -r line; do :; done\n",
            args: &[],
            metadata: MetadataShape::Backend,
        },
    )
}

#[cfg(unix)]
pub fn process_with_capabilities(compile: bool) -> RuntimeArtifact {
    process_with_frontend(capabilities_json(compile), MetadataShape::FrontendBackend)
}

#[cfg(unix)]
pub fn process_with_multi_document(installed: InstalledFrontend) -> RuntimeArtifact {
    let mut capabilities = capabilities_json(true);
    capabilities["frontend"]["multiDocument"] = serde_json::json!(true);
    process_with_frontend(capabilities, installed.metadata())
}

#[cfg(unix)]
fn process_with_frontend(
    guest_capabilities: serde_json::Value,
    metadata: MetadataShape,
) -> RuntimeArtifact {
    let guest_info = serde_json::json!({
        "id": "morphir-process-capabilities",
        "name": "Morphir Process Capabilities",
        "version": "1.2.3",
        "types": ["frontend", "backend"]
    });
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "protocolVersion": "0.1",
            "extension": guest_info,
            "capabilities": guest_capabilities
        }
    })
    .to_string();
    let program = format!(
        "#!/bin/sh\nlength=0\nwhile IFS= read -r header; do\ncase \"$header\" in Content-Length:*) length=${{header#*: }}; length=$(printf '%s' \"$length\" | tr -d '\\r') ;; esac\n[ -z \"$(printf '%s' \"$header\" | tr -d '\\r')\" ] && break\ndone\ndd bs=1 count=\"$length\" of=/dev/null 2>/dev/null\nprintf 'Content-Length: %s\\r\\n\\r\\n%s' '{}' '{}'\nwhile IFS= read -r line; do :; done\n",
        response.len(),
        response
    );
    install(
        tempfile::tempdir().unwrap(),
        ArtifactSpec {
            id: "morphir-process-capabilities",
            name: "Morphir Process Capabilities",
            runtime: "process",
            filename: "morphir-process-capabilities",
            bytes: program.as_bytes(),
            args: &[],
            metadata,
        },
    )
}

#[cfg(unix)]
pub fn process_with_frontend_workspace(compile: bool) -> RuntimeArtifact {
    let guest_info = serde_json::json!({
        "id": "morphir-process-workspace",
        "name": "Morphir Process Workspace",
        "version": "1.2.3",
        "types": ["frontend", "workspace"]
    });
    let guest_capabilities = frontend_workspace_capabilities_json(compile);
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "protocolVersion": "0.1",
            "extension": guest_info,
            "capabilities": guest_capabilities
        }
    })
    .to_string();
    let program = format!(
        "#!/bin/sh\nlength=0\nwhile IFS= read -r header; do\ncase \"$header\" in Content-Length:*) length=${{header#*: }}; length=$(printf '%s' \"$length\" | tr -d '\\r') ;; esac\n[ -z \"$(printf '%s' \"$header\" | tr -d '\\r')\" ] && break\ndone\ndd bs=1 count=\"$length\" of=/dev/null 2>/dev/null\nprintf 'Content-Length: %s\\r\\n\\r\\n%s' '{}' '{}'\nwhile IFS= read -r line; do :; done\n",
        response.len(),
        response
    );
    install(
        tempfile::tempdir().unwrap(),
        ArtifactSpec {
            id: "morphir-process-workspace",
            name: "Morphir Process Workspace",
            runtime: "process",
            filename: "morphir-process-workspace",
            bytes: program.as_bytes(),
            args: &[],
            metadata: MetadataShape::FrontendWorkspace,
        },
    )
}

pub fn wasm() -> RuntimeArtifact {
    install(
        tempfile::tempdir().unwrap(),
        ArtifactSpec {
            id: "morphir-avro",
            name: "Morphir Avro",
            runtime: "wasm",
            filename: "morphir-avro.wasm",
            bytes: &valid_guest_bytes(),
            args: &[],
            metadata: MetadataShape::FrontendBackend,
        },
    )
}

pub fn wasm_with_capabilities(compile: bool) -> RuntimeArtifact {
    wasm_with_frontend(capabilities_json(compile), MetadataShape::FrontendBackend)
}

pub fn wasm_with_multi_document(installed: InstalledFrontend) -> RuntimeArtifact {
    let mut capabilities = capabilities_json(true);
    capabilities["frontend"]["multiDocument"] = serde_json::json!(true);
    wasm_with_frontend(capabilities, installed.metadata())
}

fn wasm_with_frontend(
    guest_capabilities: serde_json::Value,
    metadata: MetadataShape,
) -> RuntimeArtifact {
    let guest_info = serde_json::json!({
        "id": "morphir-capabilities",
        "name": "Morphir Capabilities",
        "version": "1.2.3",
        "types": ["frontend", "backend"]
    });
    let bytes = guest_bytes(guest_info, guest_capabilities);
    install(
        tempfile::tempdir().unwrap(),
        ArtifactSpec {
            id: "morphir-capabilities",
            name: "Morphir Capabilities",
            runtime: "wasm",
            filename: "morphir-capabilities.wasm",
            bytes: &bytes,
            args: &[],
            metadata,
        },
    )
}

pub fn wasm_with_frontend_workspace(compile: bool) -> RuntimeArtifact {
    let guest_info = serde_json::json!({
        "id": "morphir-wasm-workspace",
        "name": "Morphir Wasm Workspace",
        "version": "1.2.3",
        "types": ["frontend", "workspace"]
    });
    let guest_capabilities = frontend_workspace_capabilities_json(compile);
    let bytes = guest_bytes(guest_info, guest_capabilities);
    install(
        tempfile::tempdir().unwrap(),
        ArtifactSpec {
            id: "morphir-wasm-workspace",
            name: "Morphir Wasm Workspace",
            runtime: "wasm",
            filename: "morphir-wasm-workspace.wasm",
            bytes: &bytes,
            args: &[],
            metadata: MetadataShape::FrontendWorkspace,
        },
    )
}

fn capabilities_json(compile: bool) -> serde_json::Value {
    serde_json::json!({
        "frontend": {
            "languages": [{"id": "gleam", "fileExtensions": [".gleam"]}],
            "irVersions": ["4"],
            "compile": compile,
            "incremental": false,
            "fragments": false
        },
        "backend": {
            "targets": ["avro", "json-schema"],
            "irVersions": ["3", "4"],
            "generate": true
        }
    })
}

fn frontend_workspace_capabilities_json(compile: bool) -> serde_json::Value {
    serde_json::json!({
        "frontend": {
            "languages": [{"id": "gleam", "fileExtensions": [".gleam"]}],
            "irVersions": ["4"],
            "compile": compile,
            "incremental": false,
            "fragments": false
        },
        "workspace": {
            "protocolVersions": ["0.1.0-draft.1"],
            "discover": true
        }
    })
}

fn install(root: TempDir, spec: ArtifactSpec<'_>) -> RuntimeArtifact {
    let index = root.path().join("index");
    let source = index.join("artifacts").join(spec.filename);
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(index.join("extensions")).unwrap();
    fs::write(&source, spec.bytes).unwrap();
    let digest = Sha256Digest::of_bytes(spec.bytes);
    let platform = Platform::current();
    let artifact = if spec.runtime == "process" {
        serde_json::json!({
            "runtime": spec.runtime,
            "platform": { "os": platform.os(), "arch": platform.arch() },
            "source": { "kind": "local-file", "path": format!("artifacts/{}", spec.filename) },
            "sha256": digest,
            "filename": spec.filename,
            "args": spec.args,
            "executable": true
        })
    } else {
        serde_json::json!({
            "runtime": spec.runtime,
            "source": { "kind": "local-file", "path": format!("artifacts/{}", spec.filename) },
            "sha256": digest,
            "filename": spec.filename
        })
    };
    let capabilities = match spec.metadata {
        #[cfg(unix)]
        MetadataShape::Backend => serde_json::json!(["backend"]),
        MetadataShape::FrontendBackend | MetadataShape::FrontendClaims(_) => {
            serde_json::json!(["frontend", "backend"])
        }
        MetadataShape::FrontendWorkspace => serde_json::json!(["frontend", "workspace"]),
    };
    let mut record = serde_json::json!({
        "schemaVersion": "1.0",
        "id": spec.id,
        "name": spec.name,
        "version": "1.2.3",
        "channels": ["stable"],
        "mepVersions": ["0.1"],
        "capabilities": capabilities,
        "artifacts": [artifact]
    });
    if matches!(
        spec.metadata,
        MetadataShape::FrontendBackend
            | MetadataShape::FrontendWorkspace
            | MetadataShape::FrontendClaims(_)
    ) {
        record.as_object_mut().unwrap().insert(
            "frontend".into(),
            serde_json::json!({
                "languages": [{"id": "gleam", "fileExtensions": [".gleam"]}],
                "irVersions": ["4"],
                "compile": true
            }),
        );
    }
    #[cfg(unix)]
    let is_backend = matches!(
        spec.metadata,
        MetadataShape::Backend | MetadataShape::FrontendBackend | MetadataShape::FrontendClaims(_)
    );
    #[cfg(not(unix))]
    let is_backend = matches!(
        spec.metadata,
        MetadataShape::FrontendBackend | MetadataShape::FrontendClaims(_)
    );

    if is_backend {
        record.as_object_mut().unwrap().insert(
            "backend".into(),
            serde_json::json!({
                "targets": ["avro", "json-schema"],
                "irVersions": ["3", "4"]
            }),
        );
    }
    if let MetadataShape::FrontendClaims(installed) = spec.metadata {
        let mut frontend = record.as_object_mut().unwrap().remove("frontend").unwrap();
        if matches!(installed, InstalledFrontend::ClaimsWithMultiDocument) {
            frontend["multiDocument"] = serde_json::json!(true);
        }
        let backend = record.as_object_mut().unwrap().remove("backend").unwrap();
        let types = record
            .as_object_mut()
            .unwrap()
            .remove("capabilities")
            .unwrap();
        record.as_object_mut().unwrap().remove("mepVersions");
        record["schemaVersion"] = serde_json::json!("2.0.0-draft.2");
        record["artifacts"][0]["claims"] = serde_json::json!({
            "claimsVersion": "0.1.0-draft.2",
            "protocolVersions": ["0.1"],
            "extension": {
                "id": spec.id, "name": spec.name, "version": "1.2.3", "types": types
            },
            "capabilities": {"frontend": frontend, "backend": backend}
        });
    }
    fs::write(
        index.join("extensions").join(format!("{}.jsonl", spec.id)),
        format!("{record}\n"),
    )
    .unwrap();

    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let extension_id = ExtensionId::parse(spec.id).unwrap();
    let selected = LocalIndex::open(&index)
        .unwrap()
        .resolve(
            &extension_id,
            Selection::Channel(Channel::Stable),
            &platform,
            &"0.4.0".parse().unwrap(),
        )
        .unwrap();
    let installed = ExtensionInstaller::new(&home)
        .install(selected, &"0.4.0".parse().unwrap())
        .unwrap();
    let installed_path = home.root().join(installed.store_path());
    #[cfg(unix)]
    let staging_directory = home.temp_dir().join("extensions");
    let artifact = activate_installed(&home, &extension_id).unwrap();
    let working_directory = root.path().join("workspace");
    fs::create_dir(&working_directory).unwrap();

    RuntimeArtifact {
        _root: root,
        artifact,
        installed_path,
        #[cfg(unix)]
        staging_directory,
        working_directory,
    }
}

fn valid_guest_bytes() -> Vec<u8> {
    let guest_info = serde_json::json!({
        "id": "guest-self-report",
        "name": "Guest Self Report",
        "version": "9.9.9",
        "types": ["validator"]
    });
    guest_bytes(guest_info, serde_json::json!({}))
}

fn guest_bytes(guest_info: serde_json::Value, guest_capabilities: serde_json::Value) -> Vec<u8> {
    let info = guest_info.to_string();
    let initialize_response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "protocolVersion": "0.1",
            "extension": guest_info,
            "capabilities": guest_capabilities
        }
    })
    .to_string();
    let writes = |output: &str| {
        output
                .bytes()
                .enumerate()
                .map(|(index, byte)| {
                    format!(
                        "(call $store_u8 (i64.add (local.get $output) (i64.const {index})) (i32.const {byte}))"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
    };
    let info_writes = writes(&info);
    let response_writes = writes(&initialize_response);
    let wat = format!(
        r#"(module
                (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
                (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
                (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
                (func (export "morphir_extension_info") (result i32)
                    (local $output i64)
                    (local.set $output (call $alloc (i64.const {info_length})))
                    {info_writes}
                    (call $output_set (local.get $output) (i64.const {info_length}))
                    (i32.const 0))
                (func (export "handle") (result i32)
                    (local $output i64)
                    (local.set $output (call $alloc (i64.const {response_length})))
                    {response_writes}
                    (call $output_set (local.get $output) (i64.const {response_length}))
                    (i32.const 0)))"#,
        info_length = info.len(),
        response_length = initialize_response.len(),
    );
    wat::parse_str(wat).unwrap()
}
