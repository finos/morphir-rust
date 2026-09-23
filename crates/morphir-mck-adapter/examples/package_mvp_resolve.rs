//! Execute one signed exact-root resolve request through the public adapter API.
//! Run with `cargo run -p morphir-mck-adapter --example package_mvp_resolve`.
use morphir_mck_adapter::package_mvp::run;
use serde_json::{Value, json};
use std::{fs, io::Cursor, path::Path};

fn fixture_files(root: &Path) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    fn collect(
        root: &Path,
        directory: &Path,
        files: &mut Vec<Value>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                collect(root, &path, files)?;
            } else if path
                .file_name()
                .is_some_and(|name| name != "bad-timestamp-signature.json")
            {
                let name = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let hex = fs::read(&path)?
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                files.push(json!({"path":name,"hex":hex}));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    Ok(files)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mvp-fresh-restore");
    let request = json!({
        "id":1,
        "op":"resolve-local-library",
        "profile":"local-library-mvp:0.1.0-draft.1",
        "exactRoot":"example.com/finance/loan-rules@1.0.0",
        "environment":{"trustState":"initialized","output":"absent"},
        "files":fixture_files(&fixture)?
    });
    let mut response = Vec::new();
    run(Cursor::new(format!("{request}\n")), &mut response)?;
    let reply: Value = serde_json::from_slice(
        response
            .split(|byte| *byte == b'\n')
            .next()
            .ok_or("missing reply")?,
    )?;
    assert_eq!(reply["outcome"], "resolved");
    assert_eq!(reply["output"], "present");
    assert_eq!(reply["outputFiles"][0]["path"], "morphir.lock");
    assert_eq!(
        reply["outputFiles"][0]["sha256"],
        "sha256:2db3c346d885528c3ef46d60ed9c563d5f6ba45992ef2f6a3c7dfc294d5b8f24"
    );
    println!("{}", reply);
    Ok(())
}
