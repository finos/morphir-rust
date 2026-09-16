//! `decode` over morphir-core, driven directly rather than through the wire.
//!
//! The Morphir Compatibility Kit itself is the oracle for what these spellings mean
//! (`spec/ir/mck` in the parent repository); nothing here reproduces a kit fence. These are the
//! adapter's own checks that the request's `version`, `path` and `strip` reach the codec and
//! that the answer carries the shape `protocol.schema.json` defines for `DecodeSuccess`.

use morphir_mck_adapter::protocol::*;
use morphir_mck_adapter::testee::decode;

fn req(node: NodeKind, input: &str, path: PathMode) -> DecodeRequest {
    DecodeRequest {
        version: 4,
        profile: Profile::Json,
        path,
        strip: true,
        node,
        input: input.into(),
    }
}

#[test]
fn a_type_variable_shorthand_decodes_to_itself() {
    match decode(&req(NodeKind::Type, "\"a\"", PathMode::Current)) {
        DecodeResponse::Ok {
            canonical,
            warnings,
            kind,
        } => {
            assert_eq!(canonical["json"], "\"a\"\n");
            assert!(warnings.is_empty());
            assert_eq!(kind, "Variable");
        }
        other => panic!("{other:?}"),
    }
}

/// `current` and `pinned` are the two module paths a binding exposes, not a spelling window, and
/// the driver holds them to the same verdict fence by fence. A legacy member therefore warns
/// identically on both.
#[test]
fn a_legacy_member_warns_the_same_way_on_both_paths() {
    let input =
        r#"{"Function":{"arg":"morphir/SDK:basics#int","result":"morphir/SDK:string#string"}}"#;
    let expected = "{ \"Function\": { \"parameterType\": \"morphir/SDK:basics#int\", \"returnType\": \"morphir/SDK:string#string\" } }\n";
    for path in [PathMode::Current, PathMode::Pinned] {
        match decode(&req(NodeKind::Type, input, path)) {
            DecodeResponse::Ok {
                warnings,
                canonical,
                ..
            } => {
                assert_eq!(warnings.len(), 2, "on {path:?}");
                assert!(
                    warnings
                        .iter()
                        .all(|w| w.code == morphir_core::ir::DiagnosticCode::LegacySpelling),
                    "on {path:?}: {warnings:?}"
                );
                assert_eq!(canonical["json"], expected, "on {path:?}");
            }
            other => panic!("on {path:?}: {other:?}"),
        }
    }
}

#[test]
fn syntax_errors_carry_the_kit_codes() {
    match decode(&req(NodeKind::Value, "{", PathMode::Current)) {
        DecodeResponse::Err { diagnostic } => {
            assert_eq!(
                diagnostic.code,
                morphir_core::ir::DiagnosticCode::InvalidJson
            )
        }
        o => panic!("{o:?}"),
    }
    match decode(&req(
        NodeKind::Value,
        r#"{"Record":{"fields":{"a":1,"a":2}}}"#,
        PathMode::Current,
    )) {
        DecodeResponse::Err { diagnostic } => {
            assert_eq!(
                diagnostic.code,
                morphir_core::ir::DiagnosticCode::DuplicateMember
            );
            assert_eq!(diagnostic.cursor, "/Record/fields/a");
        }
        o => panic!("{o:?}"),
    }
}

/// A case pinned to version 3 spells its canonical in version 3, so a version 3 decode reads and
/// writes the classic model and the fence round-trips byte for byte.
#[test]
fn a_v3_value_round_trips_in_the_classic_spelling() {
    let r = DecodeRequest {
        version: 3,
        ..req(
            NodeKind::Value,
            r#"["Literal", {}, ["WholeNumberLiteral", 42]]"#,
            PathMode::Current,
        )
    };
    match decode(&r) {
        DecodeResponse::Ok {
            canonical, kind, ..
        } => {
            assert_eq!(
                canonical["json"],
                "[\"Literal\", {}, [\"WholeNumberLiteral\", 42]]\n"
            );
            assert_eq!(kind, "Literal");
        }
        o => panic!("{o:?}"),
    }
}

/// `strip` in the classic model is writing `{}` where an inferred type was, which is the same
/// thing an untyped classic document already says.
#[test]
fn a_v3_value_with_an_inferred_type_strips_back_to_empty_attributes() {
    let typed = r#"["Variable", ["Reference", {}, [[["morphir"], ["s", "d", "k"]], [["basics"]], ["int"]], []], ["x"]]"#;
    let r = DecodeRequest {
        version: 3,
        ..req(NodeKind::Value, typed, PathMode::Current)
    };
    match decode(&r) {
        DecodeResponse::Ok { canonical, .. } => {
            assert_eq!(canonical["json"], "[\"Variable\", {}, [\"x\"]]\n")
        }
        o => panic!("{o:?}"),
    }
    let kept = DecodeRequest {
        version: 3,
        strip: false,
        ..req(NodeKind::Value, typed, PathMode::Current)
    };
    match decode(&kept) {
        DecodeResponse::Ok { canonical, .. } => assert!(
            canonical["json"].contains("Reference"),
            "the inferred type should survive strip: false, got {}",
            canonical["json"]
        ),
        o => panic!("{o:?}"),
    }
}

#[test]
fn a_duplicate_member_is_found_inside_an_array_too() {
    match decode(&req(
        NodeKind::Value,
        r#"{"Tuple":[{"Variable":"x"},{"Record":{"fields":{"a":1},"fields":{}}}]}"#,
        PathMode::Current,
    )) {
        DecodeResponse::Err { diagnostic } => {
            assert_eq!(
                diagnostic.code,
                morphir_core::ir::DiagnosticCode::DuplicateMember
            );
            assert_eq!(diagnostic.cursor, "/Tuple/1/Record/fields");
        }
        o => panic!("{o:?}"),
    }
}

/// A document literal may spell a member like serde_json's reserved number key, and an object
/// that only looks like the number token is still an object the probe has to walk.
#[test]
fn a_member_spelled_like_the_number_token_is_still_walked() {
    // Shaped like the token — one member under the reserved key — but holding an object rather
    // than a lexeme string, so the duplicate inside it has to be found.
    match decode(&req(
        NodeKind::Literal,
        r#"{"DocumentLiteral":{"$serde_json::private::Number":{"a":1,"a":2}}}"#,
        PathMode::Current,
    )) {
        DecodeResponse::Err { diagnostic } => {
            assert_eq!(
                diagnostic.code,
                morphir_core::ir::DiagnosticCode::DuplicateMember
            );
            assert_eq!(
                diagnostic.cursor,
                "/DocumentLiteral/$serde_json::private::Number/a"
            );
        }
        o => panic!("{o:?}"),
    }

    // The reserved key beside another member is a plain object too, whatever its value is.
    match decode(&req(
        NodeKind::Literal,
        r#"{"DocumentLiteral":{"$serde_json::private::Number":"1","a":1,"a":2}}"#,
        PathMode::Current,
    )) {
        DecodeResponse::Err { diagnostic } => {
            assert_eq!(
                diagnostic.code,
                morphir_core::ir::DiagnosticCode::DuplicateMember
            );
            assert_eq!(diagnostic.cursor, "/DocumentLiteral/a");
        }
        o => panic!("{o:?}"),
    }

    // And a repeat of the reserved key itself is a duplicate like any other.
    match decode(&req(
        NodeKind::Literal,
        r#"{"DocumentLiteral":{"$serde_json::private::Number":"1","$serde_json::private::Number":"2"}}"#,
        PathMode::Current,
    )) {
        DecodeResponse::Err { diagnostic } => assert_eq!(
            diagnostic.code,
            morphir_core::ir::DiagnosticCode::DuplicateMember
        ),
        o => panic!("{o:?}"),
    }

    // Nor can the reserved key be a door around the nesting ceiling: the object holding it is a
    // container, and so is everything under it.
    let input = format!(
        "{{\"$serde_json::private::Number\":{}{}}}",
        "[".repeat(MAX_DEPTH),
        "]".repeat(MAX_DEPTH)
    );
    match decode(&req(NodeKind::Value, &input, PathMode::Current)) {
        DecodeResponse::Err { diagnostic } => assert_eq!(
            diagnostic.code,
            morphir_core::ir::DiagnosticCode::NestingTooDeep
        ),
        o => panic!("{o:?}"),
    }
}

/// The ceiling the reference reader states (`MAX_DEPTH` in its JSON value layer).
const MAX_DEPTH: usize = 1000;

/// A document at or past the ceiling is answered, not crashed into.
///
/// This test spawns no thread of its own and asks for no stack: `decode` supplies the stack the
/// recursion costs, so a caller on an ordinary thread — the framing loop on the process's main
/// thread, or a test thread — gets `nesting_too_deep` back rather than a stack overflow. That is
/// the whole point of the ceiling being a stated number.
#[test]
fn a_document_deeper_than_the_nesting_limit_is_refused() {
    // One more container than the reader admits.
    let input = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
    match decode(&req(NodeKind::Value, &input, PathMode::Current)) {
        DecodeResponse::Err { diagnostic } => {
            assert_eq!(
                diagnostic.code,
                morphir_core::ir::DiagnosticCode::NestingTooDeep
            );
            assert_eq!(diagnostic.stage, morphir_core::ir::DiagnosticStage::Syntax);
        }
        o => panic!("{o:?}"),
    }
    // Exactly at the ceiling is admitted, so the limit is the boundary and not an off-by-one.
    let input = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    match decode(&req(NodeKind::Value, &input, PathMode::Current)) {
        DecodeResponse::Err { diagnostic } => assert_ne!(
            diagnostic.code,
            morphir_core::ir::DiagnosticCode::NestingTooDeep
        ),
        DecodeResponse::Ok { .. } => {}
        o => panic!("{o:?}"),
    }
    // A number is a scalar, not a container: it must not consume a nesting level. With
    // `arbitrary_precision` on, serde delivers one as a map, so this is the case that catches a
    // probe charging it depth.
    let input = format!("{}1{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    match decode(&req(NodeKind::Value, &input, PathMode::Current)) {
        DecodeResponse::Err { diagnostic } => assert_ne!(
            diagnostic.code,
            morphir_core::ir::DiagnosticCode::NestingTooDeep,
            "a number at the ceiling is not a container"
        ),
        DecodeResponse::Ok { .. } => {}
        o => panic!("{o:?}"),
    }
}

#[test]
fn strip_clears_attributes_and_no_strip_keeps_them() {
    let input = r#"{"Tuple":{"attributes":{"source":{"startLine":1,"startColumn":1,"endLine":1,"endColumn":2}},"elements":["a"]}}"#;
    match decode(&req(NodeKind::Type, input, PathMode::Current)) {
        DecodeResponse::Ok { canonical, .. } => {
            assert_eq!(canonical["json"], "{ \"Tuple\": [\"a\"] }\n")
        }
        o => panic!("{o:?}"),
    }
    let kept = DecodeRequest {
        strip: false,
        ..req(NodeKind::Type, input, PathMode::Current)
    };
    match decode(&kept) {
        DecodeResponse::Ok { canonical, .. } => assert!(
            canonical["json"].contains("startLine"),
            "attributes should survive strip: false, got {}",
            canonical["json"]
        ),
        o => panic!("{o:?}"),
    }
}

#[test]
fn an_ir_file_writes_its_format_version_first() {
    let input = r#"{"distribution":{"Library":{"packageName":"example","dependencies":{},"def":{"modules":{}}}},"formatVersion":4}"#;
    match decode(&req(NodeKind::IRFile, input, PathMode::Current)) {
        DecodeResponse::Ok {
            canonical, kind, ..
        } => {
            assert!(
                canonical["json"].starts_with("{ \"formatVersion\": 4, \"distribution\": "),
                "got {}",
                canonical["json"]
            );
            assert_eq!(kind, "Library");
        }
        o => panic!("{o:?}"),
    }
}

#[test]
fn the_distribution_node_is_the_whole_document() {
    let input = r#"{"formatVersion":4,"distribution":{"Library":{"packageName":"example","dependencies":{},"def":{"modules":{}}}}}"#;
    let file = decode(&req(NodeKind::IRFile, input, PathMode::Current));
    let distribution = decode(&req(NodeKind::Distribution, input, PathMode::Current));
    match (file, distribution) {
        (
            DecodeResponse::Ok {
                canonical: a,
                kind: ka,
                ..
            },
            DecodeResponse::Ok {
                canonical: b,
                kind: kb,
                ..
            },
        ) => {
            assert_eq!(a, b);
            assert_eq!(ka, kb);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_name_stays_a_word_array_at_version_3() {
    let r = DecodeRequest {
        version: 3,
        ..req(
            NodeKind::Name,
            r#"["value", "in", "u", "s", "d"]"#,
            PathMode::Current,
        )
    };
    match decode(&r) {
        DecodeResponse::Ok { canonical, .. } => assert_eq!(
            canonical["json"],
            "[\"value\", \"in\", \"u\", \"s\", \"d\"]\n"
        ),
        o => panic!("{o:?}"),
    }
}

/// A profile or a version outside `capabilities` is a protocol failure, not a diagnostic about
/// the document, so it never spends one of the kit's codes.
#[test]
fn an_undeclared_profile_or_version_is_refused_as_a_protocol_error() {
    let yaml = DecodeRequest {
        profile: Profile::Yaml,
        ..req(NodeKind::Type, "a", PathMode::Current)
    };
    match decode(&yaml) {
        DecodeResponse::Refused { diagnostic } => assert_eq!(diagnostic.code, "protocol_error"),
        o => panic!("{o:?}"),
    }
    let ancient = DecodeRequest {
        version: 2,
        ..req(NodeKind::Type, "\"a\"", PathMode::Current)
    };
    match decode(&ancient) {
        DecodeResponse::Refused { diagnostic } => assert_eq!(diagnostic.code, "protocol_error"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn a_node_classic_cannot_express_is_an_unknown_node_at_version_3() {
    let r = DecodeRequest {
        version: 3,
        ..req(NodeKind::FormatVersion, "3", PathMode::Current)
    };
    match decode(&r) {
        DecodeResponse::Err { diagnostic } => {
            assert_eq!(
                diagnostic.code,
                morphir_core::ir::DiagnosticCode::UnknownNode
            );
            assert_eq!(diagnostic.cursor, "/");
        }
        o => panic!("{o:?}"),
    }
}
