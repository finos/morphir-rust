use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const SELECTOR: &[&str] = &["package-mvp"];
const PROFILE: &str = "local-library-mvp:0.1.0-draft.1";

fn exchange(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mck-adapter-rust"))
        .args(SELECTOR)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mvp-fresh-restore")
}

fn files() -> Vec<Value> {
    fn collect(root: &Path, directory: &Path, found: &mut Vec<Value>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, found);
            } else {
                if path.file_name().unwrap() == "bad-timestamp-signature.json" {
                    continue;
                }
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                let bytes = fs::read(&path).unwrap();
                let hex = bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                found.push(json!({"path":name,"hex":hex}));
            }
        }
    }
    let root = fixture();
    let mut found = Vec::new();
    collect(&root, &root, &mut found);
    found.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    assert_eq!(found.len(), 15);
    found
}

fn restore(files: Vec<Value>) -> Output {
    let request = json!({"id":2,"op":"restore-local-library","profile":PROFILE,"files":files});
    exchange(&format!(
        "{{\"id\":1,\"op\":\"capabilities\"}}\n{request}\n"
    ))
}

fn responses(output: &Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

#[test]
fn advertises_only_the_mvp_restore_operation() {
    let answer = responses(&exchange("{\"id\":1,\"op\":\"capabilities\"}\n"));
    assert_eq!(
        answer,
        vec![json!({
            "id":1,"suite":"package","contractVersion":"0.1.0-draft.3",
            "implementation":"morphir-rust","implementationVersion":env!("CARGO_PKG_VERSION"),
            "profiles":[PROFILE],"operations":["restore-local-library"]
        })]
    );
}

#[test]
fn restores_signed_graph_through_production_api() {
    let answer = responses(&restore(files()));
    assert_eq!(answer.len(), 2);
    assert_eq!(
        answer[1],
        json!({
            "id":2,"outcome":"restored","output":"present",
            "lockUnchanged":true,"registryUnchanged":true,
            "packages":[
                {"packagePath":"example.com/finance/eligibility","version":"1.2.0","directory":"example.com/finance/eligibility/1.2.0"},
                {"packagePath":"example.com/finance/loan-rules","version":"1.0.0","directory":"example.com/finance/loan-rules/1.0.0"}
            ],
            "outputFiles":[
                {"path":"example.com/finance/eligibility/1.2.0/ir.json","sha256":"sha256:243e640848e5ee728224c0bc9090c67cf434d9814cec77745daad918119ceb43"},
                {"path":"example.com/finance/eligibility/1.2.0/manifest.json","sha256":"sha256:0a91b5e3a5e377fa4557212fc5b561a66067d67292123136575e46ce5a3fd0b7"},
                {"path":"example.com/finance/loan-rules/1.0.0/ir.json","sha256":"sha256:b1884eeb8364f96c6405cc45f2fe07a006c22f066a4136656ad05dda9c9a94cc"},
                {"path":"example.com/finance/loan-rules/1.0.0/manifest.json","sha256":"sha256:e98ea924e66b96a5050e2c3a690f9534db32f5971401d2965791546872ecb7a3"}
            ]
        })
    );
}

#[test]
fn invalid_timestamp_signature_is_the_narrow_refusal() {
    let mut files = files();
    let timestamp = files
        .iter_mut()
        .find(|file| file["path"] == "registry/metadata/timestamp.json")
        .unwrap();
    let invalid = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mvp-fresh-restore/bad-timestamp-signature.json"),
    )
    .unwrap();
    timestamp["hex"] = Value::String(invalid.iter().map(|byte| format!("{byte:02x}")).collect());
    let answer = responses(&restore(files));
    assert_eq!(
        answer[1],
        json!({
            "id":2,"outcome":"refused","category":"metadata-authentication",
            "reason":"timestamp-signature-threshold",
            "output":"absent","lockUnchanged":true,"registryUnchanged":true
        })
    );
}

#[test]
fn malformed_or_incomplete_input_fails_the_protocol() {
    let original = files();
    let mut missing = original.clone();
    missing.pop();
    let mut duplicate = original.clone();
    duplicate.push(original[0].clone());
    let mut traversal = original.clone();
    traversal[0]["path"] = json!("../escape");
    for files in [missing, duplicate, traversal] {
        let request = json!({"id":1,"op":"restore-local-library","profile":PROFILE,"files":files});
        let output = exchange(&format!("{request}\n"));
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let with_expected_answer = json!({
        "id":1,"op":"restore-local-library","profile":PROFILE,
        "files":original,"expected":{"outcome":"restored"}
    });
    let output = exchange(&format!("{with_expected_answer}\n"));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn invalid_lock_is_not_reported_as_a_metadata_refusal() {
    let mut files = files();
    let lock = files
        .iter_mut()
        .find(|file| file["path"] == "morphir.lock")
        .unwrap();
    lock["hex"] = json!("7b");
    let request = json!({"id":1,"op":"restore-local-library","profile":PROFILE,"files":files});
    let output = exchange(&format!("{request}\n"));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("restore failed"));
}
