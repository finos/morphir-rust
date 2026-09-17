//! The canonical YAML writer's bytes (`docs/spec/ir/schemas/v4/yaml-profile.md`, "Canonical
//! writer"). The kit's `yaml canonical` fences are the oracle; the last test holds the writer
//! against every one of them.

use morphir_core::ir::yaml::{read, write_canonical};
use serde_json::json;

fn w(v: serde_json::Value) -> String {
    write_canonical(&v)
}

#[test]
fn mappings_and_sequences() {
    assert_eq!(
        w(json!({"a": 1, "b": {"c": [1, 2], "d": {}}, "e": []})),
        "a: 1\nb:\n  c: [1, 2]\n  d: {}\ne: []\n"
    );
    assert_eq!(
        w(json!({"items": [{"x": 1}, {"y": [2, {"z": 3}]}]})),
        "items:\n  - x: 1\n  - y:\n      - 2\n      - z: 3\n"
    );
    assert_eq!(w(json!([[1, 2], [3]])), "[[1, 2], [3]]\n");
    assert_eq!(w(json!({})), "{}\n");
}

#[test]
fn quoting() {
    let cases = [
        ("hello", "hello"),
        ("", "\"\""),
        ("true", "\"true\""),
        ("null", "\"null\""),
        ("~", "\"~\""),
        ("42", "\"42\""),
        ("1.5", "\"1.5\""),
        ("0x1F", "\"0x1F\""),
        (".inf", "\".inf\""),
        ("- x", "\"- x\""),
        ("x: y", "\"x: y\""),
        ("x #y", "\"x #y\""),
        ("x:", "\"x:\""),
        ("x ", "\"x \""),
        ("#x", "\"#x\""),
        ("[x", "\"[x\""),
        ("a\nb", "\"a\\nb\""),
        ("tab\there", "\"tab\\there\""),
        ("morphir/SDK:basics#int", "morphir/SDK:basics#int"),
        ("2026-01-15", "2026-01-15"),
        ("x:y", "x:y"),
        ("a b", "a b"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            w(json!({"k": input})),
            format!("k: {expected}\n"),
            "{input:?}"
        );
    }
    // inside flow, syntax characters force quotes
    assert_eq!(
        w(json!({"k": ["a,b", "c:d", "e#f", "[g]", "plain"]})),
        "k: [\"a,b\", \"c:d\", \"e#f\", \"[g]\", plain]\n"
    );
    // keys quote by the same rule
    assert_eq!(
        w(json!({"true": 1, "a b": 2, "": 3})),
        "\"true\": 1\na b: 2\n\"\": 3\n"
    );
}

/// YAML's printable set (`c-printable`) is narrower than "not a C0 control": the C1 controls
/// other than `NEL` and the two non-characters `#xFFFE`/`#xFFFF` cannot appear in source text
/// either. A writer that printed them would emit YAML its own reader refuses.
#[test]
fn unprintable_code_points_are_escaped() {
    let escaped = [
        ("\u{80}", "\"\\u0080\""),
        ("\u{9f}", "\"\\u009f\""),
        ("\u{7f}", "\"\\u007f\""),
        ("\u{fffe}", "\"\\ufffe\""),
        ("\u{ffff}", "\"\\uffff\""),
        ("a\u{1}b", "\"a\\u0001b\""),
    ];
    for (input, expected) in escaped {
        assert_eq!(
            w(json!({ "k": input })),
            format!("k: {expected}\n"),
            "{input:?}"
        );
    }

    // `NEL` is printable in YAML 1.2, and so are the astral planes and the rest of the Latin-1
    // supplement: they are written as themselves.
    for text in ["a\u{85}b", "caf\u{e9}", "\u{1f600}", "\u{fffd}", "\u{e000}"] {
        assert_eq!(w(json!({ "k": text })), format!("k: {text}\n"), "{text:?}");
    }
}

/// What the writer writes, the reader reads back as the same string — the escapes included.
#[test]
fn unprintable_code_points_round_trip() {
    for text in [
        "\u{80}",
        "\u{9f}",
        "\u{7f}",
        "\u{fffe}",
        "\u{ffff}",
        "a\u{1}b",
        "a\u{85}b",
        "caf\u{e9}",
        "\u{1f600}",
        "\u{fffd}",
    ] {
        let document = json!({ "k": text });
        assert_eq!(read(&w(document.clone())).unwrap(), document, "{text:?}");
    }
}

#[test]
fn scalars_from_lexeme() {
    // `1e3` and `-0` are the two lexemes the reader hands on in serde_json's spelling rather
    // than the source's: `serde_json::Number` prints an exponent as `1e+3`, and folds `-0` to
    // `0`. Everything else keeps the text it was read with.
    let v = read("a: 1.50\nb: 1e3\nc: -0\nd: true\ne: null\nf: 12345678901234567890\n").unwrap();
    assert_eq!(
        w(v),
        "a: 1.50\nb: 1e+3\nc: 0\nd: true\ne: null\nf: 12345678901234567890\n"
    );
}

#[test]
fn round_trips_the_kit_style() {
    let text = "formatVersion: 4\ndistribution:\n  Library:\n    packageName: example/v4-test\n    dependencies: {}\n    def:\n      modules:\n        domain:\n          Public:\n            types: {}\n            values: {}\n";
    assert_eq!(w(read(text).unwrap()), text);
}

// ---------------------------------------------------------------------------
// The kit's fences
// ---------------------------------------------------------------------------

/// The Morphir Compatibility Kit lives in the parent repository (`spec/ir/mck` in finos/morphir),
/// which holds this one as a submodule. When this repository is checked out on its own the kit is
/// not there and the test reports that rather than failing.
fn kit_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("MORPHIR_MCK_DIR") {
        let path = std::path::PathBuf::from(dir);
        return path.is_dir().then_some(path);
    }
    let mut here = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = here.join("spec/ir/mck");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !here.pop() {
            return None;
        }
    }
}

/// Every ```` ```yaml canonical ```` fence in a kit page, with the case id whose section it sits
/// in (a `## <id>: <title>` heading).
fn fences(markdown: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut case = String::from("<no case>");
    let mut lines = markdown.lines();
    while let Some(line) = lines.next() {
        if let Some(heading) = line.strip_prefix("## ") {
            case = heading
                .split(':')
                .next()
                .unwrap_or(heading)
                .trim()
                .to_owned();
        }
        if line.trim_end() != "```yaml canonical" {
            continue;
        }
        let mut body = String::new();
        for line in lines.by_ref() {
            if line.trim_end() == "```" {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        found.push((case.clone(), body));
    }
    found
}

#[test]
fn every_kit_canonical_fence_round_trips() {
    let Some(kit) = kit_dir() else {
        eprintln!("the kit is not checked out next to this repository; skipping");
        return;
    };
    let mut total = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut pages: Vec<_> = std::fs::read_dir(&kit)
        .expect("the kit directory reads")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "md"))
        .collect();
    pages.sort();
    for page in pages {
        let text = std::fs::read_to_string(&page).expect("a kit page reads");
        for (case, fence) in fences(&text) {
            total += 1;
            let name = format!("{}#{case}", page.file_name().unwrap().to_string_lossy());
            let value = match read(&fence) {
                Ok(value) => value,
                Err(diagnostic) => {
                    failures.push(format!("{name}: the reader refused it: {diagnostic:?}"));
                    continue;
                }
            };
            let written = write_canonical(&value);
            if written == fence {
                continue;
            }
            let first_difference = written
                .lines()
                .zip(fence.lines())
                .find(|(a, b)| a != b)
                .map(|(a, b)| format!("wrote {a:?}, expected {b:?}"))
                .unwrap_or_else(|| {
                    format!(
                        "line counts differ: wrote {}, expected {}",
                        written.lines().count(),
                        fence.lines().count()
                    )
                });
            failures.push(format!("{name}: {first_difference}"));
        }
    }
    assert!(
        total > 0,
        "no `yaml canonical` fences were found in {kit:?}"
    );
    assert!(
        failures.is_empty(),
        "{} of {total} kit fences did not round-trip:\n{}",
        failures.len(),
        failures.join("\n")
    );
    eprintln!("{total} kit `yaml canonical` fences round-tripped");
}
