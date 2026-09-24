use morphir_extension_sdk::CompileRequest;
use serde_json::{Value, json};

mod test_data {
    use super::*;

    pub fn a_modern_compile_request() -> Value {
        json!({
            "languageId": "python",
            "sources": {
                "root": "file:///project/src",
                "documents": [{
                    "uri": "file:///project/src/models.py",
                    "languageId": "python",
                    "version": 1,
                    "text": "",
                }],
            },
            "package": {"name": "acme/example"},
            "options": {"typesOnly": false, "irVersion": "4"},
        })
    }
}

#[test]
fn top_level_documents_without_sources_reports_missing_sources() {
    let mut request = test_data::a_modern_compile_request();
    let sources = request.as_object_mut().unwrap().remove("sources").unwrap();
    request["documents"] = sources["documents"].clone();

    let error = serde_json::from_value::<CompileRequest>(request).unwrap_err();
    assert_eq!(error.to_string(), "missing field `sources`");
}

#[test]
fn sources_is_required_even_without_top_level_documents() {
    let mut request = test_data::a_modern_compile_request();
    request.as_object_mut().unwrap().remove("sources");

    let error = serde_json::from_value::<CompileRequest>(request).unwrap_err();
    assert_eq!(error.to_string(), "missing field `sources`");
}

#[test]
fn sources_ignores_unknown_top_level_documents() {
    let mut request = test_data::a_modern_compile_request();
    let expected: CompileRequest = serde_json::from_value(request.clone()).unwrap();

    // Unknown members are ignored regardless of their shape or contents.
    for documents in [
        json!([]),
        json!([{"uri": "file:///elsewhere.py"}]),
        json!(42),
    ] {
        request["documents"] = documents;
        let decoded: CompileRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(decoded, expected);
        assert!(
            serde_json::to_value(decoded)
                .unwrap()
                .get("documents")
                .is_none()
        );
    }
}
