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

// =============================================================================
// readTree / writeTree — version 3: the classic Specs distribution as a JSON document tree
// =============================================================================

/// morphir-core's `a_v3_specs_distribution_round_trips_through_a_tree` fixture: a `Specs` distribution
/// of one package, one module, one opaque type, laid out as a v3 document tree (every file
/// `formatVersion: "3.1.0"`).
const V3_SPECS: &str = r#"{"formatVersion":"3.1.0","distribution":["Specs",[["my"],["pkg"]],[],{"modules":[[[["basics"]],{"types":[[["int"],{"doc":"","value":["OpaqueTypeSpecification",[]]}]],"values":[],"doc":"Basics."}]]}]}"#;

/// The v3 Specs distribution laid out as a JSON document tree's files, in the wire shape a
/// `readTree` request or a `writeTree` response carries them in.
fn v3_specs_tree_files() -> Vec<serde_json::Value> {
    let distribution: morphir_core::ir::classic::Distribution =
        serde_json::from_str(V3_SPECS).expect("the fixture parses");
    let policy = morphir_core::ir::layout::TreePolicy {
        profile: morphir_core::ir::layout::Profile::Json,
        path_budget: 4000,
    };
    morphir_core::ir::layout::write_tree_v3(&distribution, &policy)
        .expect("the fixture lays out as a v3 tree")
        .into_iter()
        .map(|(path, content)| serde_json::json!({"path": path, "content": content}))
        .collect()
}

/// The classic JSON document's canonical spelling: the compact type encoding morphir-core's
/// classic model always writes, padded the way [`morphir_core::ir::json::write_canonical`] pads
/// every canonical fence, with the one trailing newline a canonical fence carries.
fn v3_specs_canonical_json() -> String {
    let distribution: morphir_core::ir::classic::Distribution =
        serde_json::from_str(V3_SPECS).expect("the fixture parses");
    let value = serde_json::to_value(&distribution).expect("a classic distribution serialises");
    format!("{}\n", morphir_core::ir::json::write_canonical(&value))
}

/// `readTree` on a version 3 tree answers `ok: true` with the classic JSON document — the same
/// shape `document-tree-0006`'s `readTree` answers for version 4 (this file's
/// `a_read_tree_request_for_the_escape_set_answers_ok_with_canonical_yaml`), but read into the
/// classic model and answered under its own canonical spelling rather than the v4 `IRFile`'s.
#[test]
fn a_read_tree_request_for_a_v3_specs_set_answers_ok_with_the_classic_json_document() {
    let line = serde_json::json!({
        "id": 4,
        "op": "readTree",
        "version": 3,
        "profile": "json",
        "path": "current",
        "strip": false,
        "node": "Distribution",
        "files": v3_specs_tree_files(),
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 4);
    assert_eq!(response["ok"], true);
    assert_eq!(response["kind"], "Specs");
    assert_eq!(response["canonical"]["json"], v3_specs_canonical_json());
    assert_eq!(response["warnings"], serde_json::json!([]));
}

/// `writeTree` on the classic JSON document answers the same files [`v3_specs_tree_files`] reads
/// from, in emission order: the manifest, the module manifest, then the type file.
#[test]
fn a_write_tree_request_for_a_v3_specs_document_answers_the_same_files() {
    let line = serde_json::json!({
        "id": 5,
        "op": "writeTree",
        "version": 3,
        "path": "current",
        "policy": {"profile": "json", "pathBudget": 4000},
        "input": v3_specs_canonical_json(),
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 5);
    assert_eq!(response["ok"], true);
    assert_eq!(
        response["files"],
        serde_json::Value::Array(v3_specs_tree_files())
    );
    // A fixed anchor that does not come from the layout code: the three files the tree holds, and
    // every one of them at "3.1.0".
    let files = response["files"].as_array().expect("files is an array");
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file["path"].as_str().expect("a path"))
        .collect();
    assert_eq!(paths.len(), 3, "{paths:?}");
    assert!(paths.contains(&"pkg/my/pkg/basics/int.type"), "{paths:?}");
    for file in files {
        let content: serde_json::Value =
            serde_json::from_str(file["content"].as_str().expect("content")).expect("JSON");
        assert_eq!(content["formatVersion"], "3.1.0", "{}", file["path"]);
    }
}

/// A version other than 3 or 4 is still refused, the way [`an_undeclared_profile_or_version_is_refused_as_a_protocol_error`]
/// (`decode.rs`) pins for `decode`.
#[test]
fn a_read_tree_request_at_an_unsupported_version_is_a_protocol_error() {
    let line = serde_json::json!({
        "id": 6,
        "op": "readTree",
        "version": 2,
        "profile": "json",
        "path": "current",
        "strip": false,
        "node": "Distribution",
        "files": [],
    });
    let response = response_to(&line.to_string());
    assert_protocol_error(&response, Some(6));
}

/// A version 3 tree's diagnostics come back through `readTree` the same way a version 4 tree's
/// do: the same [`DecodeResponse::Err`] shape, the same `WireDiagnostic` four members, and the
/// diagnostic's own code and cursor — here `layout::read_tree_v3`'s `version_mismatch` at the
/// type file's `#/formatVersion`, the way `layout_v3.rs`'s
/// `a_module_file_whose_version_differs_from_the_manifest_is_refused` pins it directly against
/// morphir-core.
#[test]
fn a_v3_tree_diagnostic_comes_back_with_its_code_and_cursor() {
    let mut files = v3_specs_tree_files();
    let node_path = "pkg/my/pkg/basics/int.type";
    for file in files.iter_mut() {
        if file["path"] == node_path {
            let mut node: serde_json::Value =
                serde_json::from_str(file["content"].as_str().unwrap()).unwrap();
            node["formatVersion"] = serde_json::json!("4.0.0");
            file["content"] =
                serde_json::Value::String(morphir_core::ir::json::write_canonical(&node));
        }
    }
    let line = serde_json::json!({
        "id": 8,
        "op": "readTree",
        "version": 3,
        "profile": "json",
        "path": "current",
        "strip": false,
        "node": "Distribution",
        "files": files,
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 8);
    assert_eq!(response["ok"], false);
    assert_eq!(response["diagnostic"]["code"], "version_mismatch");
    assert_eq!(
        response["diagnostic"]["cursor"],
        format!("{node_path}#/formatVersion")
    );
    assert_eq!(
        diagnostic_members(&response),
        ["code", "stage", "cursor", "message"]
    );
}

/// morphir-core's YAML reader is a profile of YAML, not a JSON/YAML discriminator: a *whole*
/// file spelled in canonical JSON is also syntactically valid YAML (JSON is a syntactic subset
/// of YAML), so it parses under the `yaml` profile rather than being refused by it — the same is
/// true of a v4 tree's `readTree` (verified directly against `layout::read_tree` and
/// `layout::read_tree_v3`: a whole tree written by [`morphir_core::ir::layout::write_tree_v3`]
/// under the `json` policy round-trips through `read_tree_v3(&files, Profile::Yaml)` without
/// error). There is no v4 `readTree` test pinning a refusal for this case, because there is
/// nothing today's v4 tree reader refuses it with; a v3 tree inherits the identical behaviour by
/// construction, since [`morphir_core::ir::layout::read_tree_v3`] shares `read_tree_with` with
/// [`morphir_core::ir::layout::read_tree`] (see `layout/v3_model.rs`, `layout/read.rs`). This
/// test pins that parity rather than a refusal: see the report for task 9 on the Review Focus
/// item this was meant to close.
#[test]
fn a_json_node_file_inside_a_yaml_v3_tree_parses_the_same_way_a_v4_tree_does() {
    let distribution: morphir_core::ir::classic::Distribution =
        serde_json::from_str(V3_SPECS).expect("the fixture parses");
    let yaml_policy = morphir_core::ir::layout::TreePolicy {
        profile: morphir_core::ir::layout::Profile::Yaml,
        path_budget: 4000,
    };
    let mut files = morphir_core::ir::layout::write_tree_v3(&distribution, &yaml_policy)
        .expect("the fixture lays out as a v3 yaml tree");
    let node_path = "pkg/my/pkg/basics/int.type";
    let (_, node_text) = files
        .iter()
        .find(|(path, _)| path == node_path)
        .expect("the type file is in the tree")
        .clone();
    let node_value = morphir_core::ir::layout::Profile::Yaml
        .read(&node_text)
        .expect("the yaml node parses");
    let json_spelling = morphir_core::ir::layout::Profile::Json.write(&node_value);
    for (path, content) in files.iter_mut() {
        if path == node_path {
            *content = json_spelling.clone();
        }
    }
    let files: Vec<serde_json::Value> = files
        .into_iter()
        .map(|(path, content)| serde_json::json!({"path": path, "content": content}))
        .collect();
    let line = serde_json::json!({
        "id": 7,
        "op": "readTree",
        "version": 3,
        "profile": "yaml",
        "path": "current",
        "strip": false,
        "node": "Distribution",
        "files": files,
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["id"], 7);
    // Not a refusal (see the doc comment above): the JSON-spelled node file is valid YAML too,
    // so it reads the same distribution the all-YAML tree would have.
    assert_eq!(response["ok"], true);
    assert_eq!(response["kind"], "Specs");
}

// =============================================================================
// readTree — version 3 with `strip`: a typed classic Library
// =============================================================================

/// A classic `Int` reference, the type a typed Library carries at every value position.
const V3_INT: &str = r#"["Reference",{},[[["morphir"],["s","d","k"]],[["basics"]],["int"]],[]]"#;

/// `identity x = x` with its type at every attribute position: the argument's annotation and the
/// body's own attribute.
fn v3_typed_value_definition() -> String {
    format!(
        r#"{{"inputTypes":[[["x"],{V3_INT},{V3_INT}]],"outputType":{V3_INT},"body":["Variable",{V3_INT},["x"]]}}"#
    )
}

/// A typed v3 `Library` of one module holding [`v3_typed_value_definition`].
fn v3_typed_library() -> String {
    format!(
        r#"{{"formatVersion":3,"distribution":["Library",[["my"],["pkg"]],[],{{"modules":[[[["basics"]],{{"access":"Public","value":{{"types":[],"values":[[["identity"],{{"access":"Public","value":{{"doc":"","value":{}}}}}]],"doc":null}}}}]]}}]}}"#,
        v3_typed_value_definition()
    )
}

fn v3_typed_library_tree_files() -> Vec<serde_json::Value> {
    let distribution: morphir_core::ir::classic::Distribution =
        serde_json::from_str(&v3_typed_library()).expect("the fixture parses");
    let policy = morphir_core::ir::layout::TreePolicy {
        profile: morphir_core::ir::layout::Profile::Json,
        path_budget: 4000,
    };
    morphir_core::ir::layout::write_tree_v3(&distribution, &policy)
        .expect("the fixture lays out as a v3 tree")
        .into_iter()
        .map(|(path, content)| serde_json::json!({"path": path, "content": content}))
        .collect()
}

/// The value definition inside a readTree answer for [`v3_typed_library`].
fn answered_value_definition(response: &serde_json::Value) -> serde_json::Value {
    let document: serde_json::Value = serde_json::from_str(
        response["canonical"]["json"]
            .as_str()
            .expect("a json canonical"),
    )
    .expect("the canonical parses");
    document["distribution"][3]["modules"][0][1]["value"]["values"][0][1]["value"]["value"].clone()
}

fn read_typed_library_tree(strip: bool) -> serde_json::Value {
    let line = serde_json::json!({
        "id": 9,
        "op": "readTree",
        "version": 3,
        "profile": "json",
        "path": "current",
        "strip": strip,
        "node": "Distribution",
        "files": v3_typed_library_tree_files(),
    });
    let response = response_to(&line.to_string());
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["kind"], "Library");
    response
}

/// With `strip`, a v3 tree answers its values the way `decode` answers the same value definition
/// with `strip`: every value attribute cleared to `{}`. Without it, the types stay.
#[test]
fn a_v3_read_tree_request_with_strip_clears_value_attributes() {
    let decoded = response_to(
        &serde_json::json!({
            "id": 10,
            "op": "decode",
            "version": 3,
            "profile": "json",
            "path": "current",
            "strip": true,
            "node": "ValueDefinition",
            "input": v3_typed_value_definition(),
        })
        .to_string(),
    );
    assert_eq!(decoded["ok"], true, "{decoded}");
    let decoded: serde_json::Value =
        serde_json::from_str(decoded["canonical"]["json"].as_str().unwrap()).unwrap();
    assert_eq!(decoded["body"][1], serde_json::json!({}));

    let stripped = read_typed_library_tree(true);
    assert_eq!(answered_value_definition(&stripped), decoded);

    let kept = read_typed_library_tree(false);
    let int: serde_json::Value = serde_json::from_str(V3_INT).unwrap();
    assert_eq!(answered_value_definition(&kept)["body"][1], int);
}
