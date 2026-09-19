use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::GleamExtension;

fn request(body: &str, version: &str, types_only: bool) -> CompileRequest {
    CompileRequest {
        language_id: "gleam".into(),
        documents: vec![SourceDocument {
            uri: "file:///workspace/src/main.gleam".into(),
            language_id: "gleam".into(),
            version: 1,
            text: format!("pub type Model = Int\npub fn run(value) {{ {body} }}"),
        }],
        package: CompilePackage {
            name: "example/package".into(),
            exposed_modules: None,
        },
        options: CompileOptions {
            ir_version: version.into(),
            types_only,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn malformed_patterns_are_parse_errors_even_when_values_are_skipped() {
    for body in [
        "let = value\n 1",
        "let assert = value\n 1",
        "let #(x,, y) = value\n 1",
        "let [x,, y] = value\n 1",
        "let Some(,) = value\n 1",
        "use #(x,, y) <- value\n 1",
        "case value { -> 1 }",
        "case value { 1 | -> 1 }",
        "case value { #(x,, y) -> 1 }",
        "case value { [x,, y] -> 1 }",
        "case value { Some(,) -> 1 }",
        "case value { \"prefix\" <> -> 1 }",
        "case value { <<x:size()>> -> 1 }",
    ] {
        for (version, types_only) in [("4", false), ("4", true), ("3", false)] {
            let result = GleamExtension
                .compile(request(body, version, types_only))
                .unwrap();
            assert!(
                !result.success,
                "must reject malformed pattern in IR {version}, types_only={types_only}: {body}"
            );
            assert!(result.ir.is_none());
            let diagnostic = result
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code.as_deref() == Some("PARSE_ERROR"))
                .unwrap_or_else(|| {
                    panic!("expected parse error for {body}: {:?}", result.diagnostics)
                });
            let location = diagnostic.location.as_ref().expect("parse error location");
            assert_eq!(location.uri, "file:///workspace/src/main.gleam");
            assert_eq!(location.range.start.line, 1);
        }
    }
}

#[test]
fn valid_unsupported_patterns_remain_skippable() {
    for body in [
        "let assert \"prefix\" <> rest = value\n rest",
        "case value { \"prefix\" <> rest -> rest }",
        "let assert <<x:size(8)>> = value\n x",
        "case value { <<x:size(8)>> -> x }",
    ] {
        for (version, types_only) in [("4", true), ("3", false)] {
            let result = GleamExtension
                .compile(request(body, version, types_only))
                .unwrap();
            assert!(result.success, "{body}: {:?}", result.diagnostics);
            assert!(result.ir.is_some());
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code.as_deref() == Some("GLEAM_VALUE_SKIPPED"))
            );
        }
    }
}
