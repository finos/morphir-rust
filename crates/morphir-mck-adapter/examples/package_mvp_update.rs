//! Execute one scoped local Library update through the public adapter API.
//! Run with `cargo run -p morphir-mck-adapter --example package_mvp_update`.
use morphir_mck_adapter::package_mvp::run;
use serde_json::{Value, json};
use std::{fs, io::Cursor, path::Path};

fn collect(root: &Path, path: &Path, files: &mut Vec<Value>) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(root, &entry?.path(), files)?;
        }
    } else {
        let name = path.strip_prefix(root).expect("fixture member");
        let hex = fs::read(path)?
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        files.push(
            json!({"path":name.to_str().expect("UTF-8 fixture path").replace('\\', "/"),"hex":hex}),
        );
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../morphir-package/tests/local_registry/update-fixture");
    let mut files = Vec::new();
    for name in ["morphir.lock", "trust-policy.json", "registry"] {
        collect(&root, &root.join(name), &mut files)?;
    }
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let request = json!({
        "id":1,"op":"update-local-library",
        "profile":"local-library-mvp:0.1.0-draft.1",
        "targets":["example.com/finance/eligibility"],
        "environment":{"trustState":"initialized","output":"absent"},
        "files":files
    });
    let mut response = Vec::new();
    run(Cursor::new(format!("{request}\n")), &mut response)?;
    let reply: Value = serde_json::from_slice(
        response
            .split(|byte| *byte == b'\n')
            .next()
            .ok_or("missing reply")?,
    )?;
    assert_eq!(reply["outcome"], "updated");
    assert_eq!(
        reply["outputFiles"][0]["sha256"],
        "sha256:7190428c25cd4b927c5db8bb9fbcbe936c81cfacc679734309a76424d332321a"
    );
    println!("{reply}");
    Ok(())
}
