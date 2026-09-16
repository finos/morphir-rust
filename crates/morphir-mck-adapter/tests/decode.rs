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

#[test]
fn a_legacy_member_warns_at_current_and_fails_at_pinned() {
    let input =
        r#"{"Function":{"arg":"morphir/SDK:basics#int","result":"morphir/SDK:string#string"}}"#;
    match decode(&req(NodeKind::Type, input, PathMode::Current)) {
        DecodeResponse::Ok {
            warnings,
            canonical,
            ..
        } => {
            assert_eq!(warnings.len(), 2);
            assert_eq!(
                canonical["json"],
                "{ \"Function\": { \"parameterType\": \"morphir/SDK:basics#int\", \"returnType\": \"morphir/SDK:string#string\" } }\n"
            );
        }
        other => panic!("{other:?}"),
    }
    match decode(&req(NodeKind::Type, input, PathMode::Pinned)) {
        DecodeResponse::Err { diagnostic } => assert_eq!(
            diagnostic.code,
            morphir_core::ir::DiagnosticCode::UnknownMember
        ),
        other => panic!("{other:?}"),
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

#[test]
fn a_v3_literal_decodes_through_migration() {
    let r = DecodeRequest {
        version: 3,
        ..req(
            NodeKind::Value,
            r#"["Literal", {}, ["WholeNumberLiteral", 42]]"#,
            PathMode::Current,
        )
    };
    match decode(&r) {
        DecodeResponse::Ok { canonical, .. } => {
            assert_eq!(
                canonical["json"],
                "{ \"Literal\": { \"IntegerLiteral\": 42 } }\n"
            )
        }
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

/// Both halves of the nesting check run on a thread with room for the recursion a document at
/// the ceiling actually costs: the decoders recurse once per level, and an unoptimized build's
/// frames are wide enough that the default test-thread stack is not enough for 512 of them.
#[test]
fn a_document_deeper_than_the_nesting_limit_is_refused() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(nesting_limit)
        .expect("spawn")
        .join()
        .expect("join");
}

fn nesting_limit() {
    // 513 nested arrays: one more than the limit the reader admits.
    let input = format!("{}{}", "[".repeat(513), "]".repeat(513));
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
    // 512 is admitted, so the limit is the boundary and not an off-by-one.
    let input = format!("{}{}", "[".repeat(512), "]".repeat(512));
    match decode(&req(NodeKind::Value, &input, PathMode::Current)) {
        DecodeResponse::Err { diagnostic } => assert_ne!(
            diagnostic.code,
            morphir_core::ir::DiagnosticCode::NestingTooDeep
        ),
        DecodeResponse::Ok { .. } => {}
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
fn a_name_decodes_at_version_3_through_the_legacy_word_array() {
    let r = DecodeRequest {
        version: 3,
        ..req(
            NodeKind::Name,
            r#"["value","in","u","s","d"]"#,
            PathMode::Current,
        )
    };
    match decode(&r) {
        DecodeResponse::Ok { canonical, .. } => {
            assert_eq!(canonical["json"], "\"value-in-USD\"\n")
        }
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
