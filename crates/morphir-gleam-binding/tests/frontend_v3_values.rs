use morphir_core::ir::classic::{self, DistributionBody, Value};
use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::GleamExtension;

#[test]
fn v3_compilation_retains_function_bodies_and_types_only_omits_them() {
    let source = "pub fn count(items: List(Int)) -> Int { case items { [] -> 0 [_, ..rest] -> 1 + count(rest) } }";
    for types_only in [false, true] {
        let result = GleamExtension
            .compile(CompileRequest {
                language_id: "gleam".into(),
                sources: SourceSet {
                    root: None,
                    documents: vec![SourceDocument {
                        uri: "file:///src/main.gleam".into(),
                        language_id: "gleam".into(),
                        version: 1,
                        text: source.into(),
                    }],
                },
                package: CompilePackage {
                    name: "example/arity".into(),
                    exposed_modules: None,
                },
                options: CompileOptions {
                    ir_version: "3".into(),
                    types_only,
                    extra: [("emitParseStage".into(), false.into())].into(),
                },
                ..Default::default()
            })
            .expect("compile request");
        assert!(result.success, "{:?}", result.diagnostics);
        let distribution: classic::Distribution =
            serde_json::from_value(result.ir.expect("V3 distribution")).unwrap();
        let DistributionBody::Library(_, _, package) = distribution.distribution;
        let values = &package.modules[0].definition.value.values;
        if types_only {
            assert!(values.is_empty());
        } else {
            assert_eq!(values.len(), 1);
            assert!(matches!(
                values[0].1.value.value.body,
                Value::PatternMatch(..)
            ));
            assert!(
                !result.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code.as_deref() == Some("GLEAM_VALUE_SKIPPED")
                })
            );
        }
    }
}

#[test]
fn v3_compilation_rejects_missing_explicit_sdk_interface() {
    let result = GleamExtension
        .compile(CompileRequest {
            language_id: "gleam".into(),
            sources: SourceSet {
                root: None,
                documents: vec![SourceDocument {
                    uri: "file:///src/main.gleam".into(),
                    language_id: "gleam".into(),
                    version: 1,
                    text: "import gleam/dict.{type Dict}\npub type Wrapper { Wrapper(Dict(Int, Int)) }".into(),
                }],
            },
            package: CompilePackage { name: "example/arity".into(), exposed_modules: None },
            options: CompileOptions {
                ir_version: "3".into(),
                types_only: false,
                extra: [("emitParseStage".into(), false.into())].into(),
            },
            ..Default::default()
        })
        .unwrap();
    assert!(!result.success);
    assert!(result.ir.is_none());
    assert!(
        result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_deref() == Some("GLEAM_TYPED_ANALYSIS")
                && diagnostic
                    .message
                    .contains("Missing explicit morphir/SDK dependency")
        }),
        "{:?}",
        result.diagnostics
    );
}
