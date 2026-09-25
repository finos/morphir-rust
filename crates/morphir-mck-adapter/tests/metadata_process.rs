use base64::Engine as _;
use morphir_mck_adapter::metadata::run;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn exchange(left: Value, right: Value, closure: Value) -> Vec<Value> {
    let bytes = serde_json::to_vec(&closure).unwrap();
    let fixture = json!({"path":"metadata-fixtures/schema-closure.json",
        "sha256":Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "contentBase64":base64::engine::general_purpose::STANDARD.encode(bytes)});
    let requests = [
        json!({"id":1,"op":"capabilities"}),
        json!({"id":2,"op":"run","caseId":"metadata-0061","operation":"compareFacts",
            "targets":[{"profile":"json","layout":"single","irRevision":"4.1.0"}],
            "given":{"left":left,"right":right},
            "schemaClosure":"metadata-fixtures/schema-closure.json","fixtures":[fixture]}),
        json!({"id":3,"op":"exit"}),
    ];
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let mut output = Vec::new();
    run(input.as_bytes(), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn compare_facts_claim_uses_real_context_expansion_and_json_datatype() {
    let predicate = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/display-info";
    let datatype = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/type/display-info";
    let subject = "morphir://ir/pkg/acme/orders?format=4.1.0#/module/api";
    let left = json!({"owner":subject,"carrier":"attributesFacts",
        "context":{"display":{"@id":predicate,"@type":"@json"}},
        "facts":{"display":{"title":"Order","enabled":true}}});
    let right = json!({"owner":subject,"carrier":"attributesFacts",
        "context":{"display":{"@id":predicate,"@type":"@json"}},
        "facts":{"display":{"enabled":true,"title":"Order"}}});
    let closure =
        json!({"predicates":[{"uri":predicate,"object":{"kind":"json","type":datatype}}]});
    let replies = exchange(left, right, closure);
    assert_eq!(replies[0]["suite"], "metadata");
    assert_eq!(
        replies[0]["claims"],
        json!([{"operation":"compareFacts","profile":"json","layout":"single","irRevision":"4.1.0"}])
    );
    assert_eq!(replies[1]["ok"], true);
    assert_eq!(replies[1]["observation"]["equal"], true);
    assert_eq!(replies[1]["observation"]["distinctFacts"], 1);
    assert_eq!(
        replies[1]["observation"]["facts"][0]["object"],
        json!({"@value":{"title":"Order","enabled":true},"@type":"@json"})
    );
}

#[test]
fn compare_facts_keeps_json_array_order_distinct() {
    let predicate = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/label-list";
    let datatype = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/type/label-list";
    let subject = "morphir://ir/pkg/acme/orders?format=4.1.0#/module/api";
    let item = |values| {
        json!({"owner":subject,"carrier":"attributesFacts",
        "context":{"labels":{"@id":predicate,"@type":"@json"}},"facts":{"labels":values}})
    };
    let closure =
        json!({"predicates":[{"uri":predicate,"object":{"kind":"json","type":datatype}}]});
    let replies = exchange(
        item(json!(["alpha", "beta"])),
        item(json!(["beta", "alpha"])),
        closure,
    );
    assert_eq!(replies[1]["observation"]["equal"], false);
    assert_eq!(replies[1]["observation"]["distinctFacts"], 2);
}
