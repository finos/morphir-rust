use morphir_mck_adapter::protocol::*;
use morphir_mck_adapter::runtime::run;

#[test]
fn a_decode_request_parses_and_unknown_fields_are_refused() {
    let (id, req) = parse_line(r#"{"id":2,"op":"decode","version":4,"profile":"json","path":"current","strip":true,"node":"Type","input":"\"a\""}"#).unwrap();
    assert_eq!(id, 2);
    assert!(matches!(req, Request::Decode(d) if d.node == NodeKind::Type && d.strip));
    let (id, diag) = parse_line(r#"{"id":3,"op":"decode","version":4,"profile":"json","path":"current","strip":true,"node":"Type","input":"","extra":1}"#).unwrap_err();
    assert_eq!(id, Some(3));
    assert_eq!(diag.code, "protocol_error");
}

#[test]
fn malformed_lines_report_invalid_json_without_an_id() {
    let (id, diag) = parse_line("{not json").unwrap_err();
    assert_eq!(id, None);
    assert_eq!(diag.code, "protocol_error");
}

#[test]
fn capabilities_match_the_stage_one_contract() {
    let caps = serde_json::to_value(capabilities()).unwrap();
    assert_eq!(caps["contractVersion"], 1);
    assert_eq!(caps["binding"], "morphir-rust");
    assert_eq!(caps["language"], "rust");
    assert_eq!(caps["formatVersions"], "[3.0.0,3.2.0),[4.0.0,4.1.0)");
    assert_eq!(caps["versions"], serde_json::json!([3, 4]));
    assert_eq!(caps["profiles"], serde_json::json!(["json", "yaml"]));
    assert_eq!(caps["layouts"], serde_json::json!(["single", "tree"]));
    assert_eq!(caps["paths"], serde_json::json!(["current", "pinned"]));
    // The eighteen nodes of a single document, plus the four files a document tree is made of.
    assert_eq!(caps["nodes"].as_array().unwrap().len(), 22);
    for file in [
        "DistributionManifestFile",
        "ModuleManifestFile",
        "TypeDefinitionFile",
        "ValueDefinitionFile",
    ] {
        assert!(
            caps["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node == file),
            "{file}"
        );
    }
}

/// `protocol.schema.json` requires `formatVersions` in the capabilities reply,
/// and the driver reads it as this binding's support table: the canonical
/// spelling of `SupportTable::reference()`. The member follows `language` on
/// the wire, which is the order the schema's worked example shows.
#[test]
fn the_capabilities_reply_declares_the_support_table() {
    let line = serde_json::to_string(&capabilities()).unwrap();
    assert!(
        line.contains(r#""formatVersions":"[3.0.0,3.2.0),[4.0.0,4.1.0)""#),
        "{line}"
    );
    assert!(
        line.contains(r#""language":"rust","formatVersions":"#),
        "formatVersions must follow language: {line}"
    );
}

/// Runs the framing loop over a single input line and returns the one
/// response line it wrote, parsed back into JSON.
fn response_to(line: &str) -> serde_json::Value {
    let mut output = Vec::new();
    run(line.as_bytes(), &mut output).expect("run");
    let text = String::from_utf8(output).expect("utf8 output");
    let text = text.trim();
    assert!(!text.is_empty(), "expected one response line, got none");
    serde_json::from_str(text).expect("response is JSON")
}

/// Every protocol-level failure answers the same shape on stdout: `id` is
/// the id when one parsed, else `null`, and the diagnostic is always
/// `protocol_error`/`syntax`/`/` with a message describing what was wrong.
fn assert_protocol_error(response: &serde_json::Value, expected_id: Option<u64>) {
    match expected_id {
        Some(id) => assert_eq!(response["id"], id),
        None => assert!(
            response["id"].is_null(),
            "expected id: null, got {response}"
        ),
    }
    assert_eq!(response["ok"], false);
    let diagnostic = &response["diagnostic"];
    assert_eq!(diagnostic["code"], "protocol_error");
    assert_eq!(diagnostic["stage"], "syntax");
    assert_eq!(diagnostic["cursor"], "/");
    assert!(
        diagnostic["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "expected a non-empty message, got {response}"
    );
}

#[test]
fn a_line_that_is_valid_json_but_not_an_object_is_a_protocol_error() {
    let response = response_to("[1,2]");
    assert_protocol_error(&response, None);
}

#[test]
fn a_non_integer_id_is_a_protocol_error() {
    let response = response_to(r#"{"id":"2","op":"capabilities"}"#);
    assert_protocol_error(&response, None);
}

#[test]
fn an_empty_op_is_a_protocol_error() {
    let response = response_to(r#"{"id":5,"op":""}"#);
    assert_protocol_error(&response, Some(5));
}

#[test]
fn an_id_of_zero_is_a_protocol_error() {
    let response = response_to(r#"{"id":0,"op":"capabilities"}"#);
    assert_protocol_error(&response, None);
}

#[test]
fn a_line_that_is_not_json_at_all_is_a_protocol_error() {
    let response = response_to("{not json");
    assert_protocol_error(&response, None);
}

/// A document nested past the profile's ceiling is answered over the wire, on whatever stack the
/// framing loop happens to be on — here the default test-thread stack, in the binary the process
/// main thread. The decoders recurse once per nesting level, so without `decode` stating the
/// stack it runs on, a conforming-looking document could abort the process instead of getting
/// the `nesting_too_deep` the profile promises.
#[test]
fn a_document_nested_past_the_ceiling_answers_over_the_wire() {
    // One container more than the reference reader's `MAX_DEPTH`.
    let depth = 1001;
    let document = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let line = serde_json::json!({
        "id": 1,
        "op": "decode",
        "version": 4,
        "profile": "json",
        "path": "current",
        "strip": true,
        "node": "Value",
        "input": document,
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 1);
    assert_eq!(response["ok"], false);
    assert_eq!(response["diagnostic"]["code"], "nesting_too_deep");
    assert_eq!(response["diagnostic"]["stage"], "syntax");
}

/// `protocol.schema.json`'s `Diagnostic` is `additionalProperties: false`, and morphir-core's
/// diagnostic carries the line and column a syntax failure was found at. A response carrying
/// them is not merely verbose — the driver cannot read it at all, and declares the adapter
/// unavailable for the rest of the run. Every YAML diagnostic is located, so this is the whole
/// yaml profile's adjudication.
#[test]
fn a_located_diagnostic_is_answered_without_its_line_and_column() {
    let line = serde_json::json!({
        "id": 1,
        "op": "decode",
        "version": 4,
        "profile": "yaml",
        "path": "current",
        "strip": true,
        "node": "Literal",
        "input": "IntegerLiteral: 0o17\n",
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["ok"], false);
    assert_eq!(response["diagnostic"]["code"], "invalid_literal");
    assert_eq!(
        diagnostic_members(&response),
        ["code", "stage", "cursor", "message"]
    );
}

/// The JSON profile answers the same four-member diagnostic. The wire shape changed for both
/// profiles, and the JSON one is what the kit adjudicates most of its fences through, so it is
/// held to the shape too rather than inheriting the YAML case's guarantee.
#[test]
fn a_json_diagnostic_is_answered_with_the_same_four_members() {
    let line = serde_json::json!({
        "id": 1,
        "op": "decode",
        "version": 4,
        "profile": "json",
        "path": "current",
        "strip": true,
        "node": "Literal",
        "input": "{\"IntegerLiteral\": ",
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["ok"], false);
    assert_eq!(response["diagnostic"]["code"], "invalid_json");
    assert_eq!(
        diagnostic_members(&response),
        ["code", "stage", "cursor", "message"]
    );
}

/// The members of a response's diagnostic, in the order it wrote them.
fn diagnostic_members(response: &serde_json::Value) -> Vec<&String> {
    response["diagnostic"]
        .as_object()
        .expect("a diagnostic object")
        .keys()
        .collect()
}

// =============================================================================
// readTree / writeTree — kit `document-tree-0003`'s `escape` set
// =============================================================================

const ESCAPE_MANIFEST: &str =
    "formatVersion: 4\ndistribution: Library\npackage: my-org/my-project\npathBudget: 4000\n";
const ESCAPE_MODULE: &str = "formatVersion: 4\npath: domain\ntypes: [user-ID]\nvalues: []\n";
const ESCAPE_NODE: &str = "formatVersion: 4\nname: user-ID\ndef:\n  Public:\n    doc: The user's identifier\n    TypeAliasDefinition:\n      typeParams: []\n      typeExp: morphir/SDK:string#string\n";
const ESCAPE_CANONICAL: &str = "formatVersion: 4\ndistribution:\n  Library:\n    packageName: my-org/my-project\n    dependencies: {}\n    def:\n      modules:\n        domain:\n          Public:\n            types:\n              user-ID:\n                Public:\n                  doc: The user's identifier\n                  TypeAliasDefinition:\n                    typeParams: []\n                    typeExp: morphir/SDK:string#string\n            values: {}\n";

/// `readTree` on kit `document-tree-0003`'s `escape` set answers `ok: true` with the case's own
/// `canonical.yaml` fence — the whole point of declaring the `tree` layout.
#[test]
fn a_read_tree_request_for_the_escape_set_answers_ok_with_canonical_yaml() {
    let line = serde_json::json!({
        "id": 1,
        "op": "readTree",
        "version": 4,
        "profile": "yaml",
        "path": "current",
        "strip": false,
        "node": "Distribution",
        "files": [
            {"path": "manifest", "content": ESCAPE_MANIFEST},
            {"path": "pkg/my-org/my-project/domain/module", "content": ESCAPE_MODULE},
            {"path": "pkg/my-org/my-project/domain/user-_id.type", "content": ESCAPE_NODE},
        ],
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 1);
    assert_eq!(response["ok"], true);
    assert_eq!(response["kind"], "Library");
    assert_eq!(response["canonical"]["yaml"], ESCAPE_CANONICAL);
    assert_eq!(response["warnings"], serde_json::json!([]));
}

/// `writeTree` on the same set answers `files` with the three logical paths in emission order:
/// the manifest, then the module's own manifest, then its type file.
#[test]
fn a_write_tree_request_for_the_escape_set_answers_files_in_emission_order() {
    let line = serde_json::json!({
        "id": 2,
        "op": "writeTree",
        "version": 4,
        "path": "current",
        "policy": {"profile": "yaml", "pathBudget": 4000},
        "input": ESCAPE_CANONICAL,
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 2);
    assert_eq!(response["ok"], true);
    let files = response["files"].as_array().expect("a files array");
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file["path"].as_str().expect("a path"))
        .collect();
    assert_eq!(
        paths,
        vec![
            "manifest",
            "pkg/my-org/my-project/domain/module",
            "pkg/my-org/my-project/domain/user-_id.type",
        ]
    );
    assert_eq!(files[0]["content"], ESCAPE_MANIFEST);
    assert_eq!(files[1]["content"], ESCAPE_MODULE);
    assert_eq!(files[2]["content"], ESCAPE_NODE);
}

/// A `writeTree` whose `input` is not a document at all answers `ok: false` with the same
/// four-member diagnostic shape every other refusal in this protocol carries.
#[test]
fn a_write_tree_request_whose_input_is_not_a_document_answers_ok_false() {
    let line = serde_json::json!({
        "id": 3,
        "op": "writeTree",
        "version": 4,
        "path": "current",
        "policy": {"profile": "yaml", "pathBudget": 4000},
        "input": "42\n",
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 3);
    assert_eq!(response["ok"], false);
    assert_eq!(
        diagnostic_members(&response),
        ["code", "stage", "cursor", "message"]
    );
}
