use serde_json::{Value, json};

const V3: &str = include_str!("../../morphir-core/tests/fixtures/ir/classic/greeting-example.json");
const V4: &str =
    include_str!("../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json");

fn exchange(requests: &[Value]) -> Vec<Value> {
    let mut input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    input.push('\n');
    let mut output = Vec::new();
    morphir_mck_adapter::node_address::run(input.as_bytes(), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn node_address_contract_resolves_v3_v4_and_reports_stale_targets() {
    let replies = exchange(&[
        json!({"id":1,"op":"capabilities"}),
        json!({"id":2,"op":"resolve","input":V3,"uri":"morphir://ir/pkg/elm-compat?format=3.0.0#/module/api/type/request"}),
        json!({"id":3,"op":"resolve","input":V4,"uri":"morphir://ir/pkg/example/v4-test?format=4.0.0#/module/domain/type/user-id"}),
        json!({"id":4,"op":"resolve","input":V4,"uri":"morphir://ir/pkg/example/v4-test?format=4.0.0#/module/domain/type/missing"}),
    ]);
    assert_eq!(replies[0]["contractVersion"], "0.1.0-draft.1");
    assert_eq!(replies[0]["operations"], json!(["resolve"]));
    assert_eq!(replies[1]["outcome"], "resolved");
    assert_eq!(replies[1]["kind"], "TypeDefinition");
    assert_eq!(replies[2]["outcome"], "resolved");
    assert_eq!(replies[3]["outcome"], "stale_target");
}

#[test]
fn malformed_uri_and_absent_revision_are_distinct() {
    let replies = exchange(&[
        json!({"id":1,"op":"resolve","input":V4,"uri":"invalid"}),
        json!({"id":2,"op":"resolve","input":V4,"uri":format!("morphir://ir/pkg/example/v4-test?format=4.0.0&rev=sha256:{}#/module/domain/type/user-id", "a".repeat(64))}),
    ]);
    assert_eq!(replies[0]["outcome"], "invalid_node_uri");
    assert_eq!(replies[1]["outcome"], "revision_unavailable");
}

#[test]
fn v3_specs_public_module_is_addressable() {
    let specs = json!({
        "formatVersion": "3.1.0",
        "distribution": ["Specs", [["acme"]], [], {
            "modules": [[[["domain"]], {"types": [], "values": [], "doc": null}]]
        }]
    });
    let replies = exchange(&[json!({
        "id": 1,
        "op": "resolve",
        "input": specs.to_string(),
        "uri": "morphir://ir/pkg/acme?format=3.1.0#/module/domain"
    })]);
    assert_eq!(replies[0]["outcome"], "resolved");
    assert_eq!(replies[0]["kind"], "Module");

    let patch_release = specs.to_string().replace("3.1.0", "3.1.1");
    let patched = exchange(&[json!({
        "id": 2,
        "op": "resolve",
        "input": patch_release,
        "uri": "morphir://ir/pkg/acme?format=3.1.1#/module/domain"
    })]);
    assert_eq!(patched[0]["outcome"], "resolved");
}
