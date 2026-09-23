//! Execute one metadata-only refresh through the public adapter API.
//! Run with `cargo run -p morphir-mck-adapter --example package_mvp_refresh`.
use morphir_mck_adapter::package_mvp::run;
use serde_json::{Value, json};
use std::{fs, io::Cursor, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mvp-fresh-restore");
    let files = [
        "morphir.lock",
        "trust-policy.json",
        "registry/metadata/1.root.json",
        "registry/metadata/1.snapshot.json",
        "registry/metadata/1.targets.json",
        "registry/metadata/1.timestamp.json",
        "registry/metadata/timestamp.json",
    ]
    .into_iter()
    .map(|path| {
        let hex = fs::read(fixture.join(path))?
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok::<Value, std::io::Error>(json!({"path":path,"hex":hex}))
    })
    .collect::<Result<Vec<_>, _>>()?;
    let request = json!({
        "id":1,"op":"refresh-local-library",
        "profile":"local-library-mvp:0.1.0-draft.1",
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
    assert_eq!(reply["outcome"], "refreshed");
    assert_eq!(reply["output"], "absent");
    assert_eq!(
        reply["receipt"]["timestampDigest"],
        "sha256:42ef09963e40de1cff6de42d5680d4c67e9f337693fb804af2873f5ee4d19636"
    );
    println!("{}", reply);
    Ok(())
}
