use super::*;
use pretty_assertions::assert_eq;

pub(crate) fn golden(case: &GoldenCase, actual: &str) -> String {
    golden_content(case.golden, actual)
}

pub(crate) fn golden_content(name: &str, actual: &str) -> String {
    let path = golden_path(name);
    let update = golden_update_mode(
        env::var_os("UPDATE_GOLDEN").as_deref(),
        env::var_os("CI").as_deref(),
    )
    .unwrap_or_else(|message| panic!("{message}"));
    if update {
        fs::create_dir_all(path.parent().expect("golden path has a parent"))
            .expect("create golden directory");
        fs::write(&path, actual).expect("write golden");
    }
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "read {}: {error}; review with UPDATE_GOLDEN=1",
            path.display()
        )
    })
}

pub(crate) fn golden_update_mode(
    update_golden: Option<&OsStr>,
    ci: Option<&OsStr>,
) -> Result<bool, &'static str> {
    let update = update_golden == Some(OsStr::new("1"));
    if update && ci.is_some() {
        return Err("refusing to update goldens in CI");
    }
    Ok(update)
}

pub(crate) fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

pub(crate) fn validate_json_artifact(path: &str, content: &str) {
    let value: Value = serde_json::from_str(content).expect("valid JSON");
    if path.ends_with(".avsc") {
        Schema::parse_str(content).expect("Apache Avro accepts schema");
        return;
    }

    let object = value.as_object().expect("protocol JSON object");
    assert!(object.get("protocol").is_some());
    assert!(object.get("namespace").is_some());
    validate_protocol_registry(&value);
    let messages = object["messages"]
        .as_object()
        .expect("protocol messages object");
    for message in messages.values() {
        assert!(message["request"].is_array());
        assert!(message.get("response").is_some());
        assert_eq!(message["errors"], Value::Array(Vec::new()));
    }
}

pub(crate) fn only_protocol_json(artifacts: &[morphir_extension_sdk::Artifact]) -> Value {
    let protocol = artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with(".avpr"))
        .expect("protocol artifact");
    serde_json::from_str(&protocol.content).expect("valid protocol JSON")
}

pub(crate) fn validate_protocol_registry(protocol: &Value) {
    validate_protocol_registry_with_linked(protocol, std::iter::empty::<&str>());
}

pub(crate) fn validate_protocol_registry_with_linked<'a>(
    protocol: &Value,
    linked_schemas: impl IntoIterator<Item = &'a str>,
) {
    let types = protocol["types"].as_array().expect("protocol types array");
    let mut type_strings = linked_schemas
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    type_strings.extend(types.iter().map(Value::to_string));
    let parsed = Schema::parse_list(&type_strings).expect("Apache Avro accepts protocol type set");
    let registry = ResolvedSchema::new_with_schemata(parsed.iter().collect())
        .expect("Apache Avro resolves protocol type registry");
    let messages = protocol["messages"]
        .as_object()
        .expect("protocol messages object");
    for message in messages.values() {
        for field in message["request"].as_array().expect("request fields") {
            resolve_protocol_type(&field["type"], registry.get_names());
        }
        resolve_protocol_type(&message["response"], registry.get_names());
        for error in message["errors"].as_array().expect("error types") {
            resolve_protocol_type(error, registry.get_names());
        }
    }
}

pub(crate) fn resolve_protocol_type(tpe: &Value, registry: &NamesRef<'_>) {
    match tpe {
        Value::String(name) if !is_primitive(name) => {
            let name = Name::new(name).expect("valid Avro named reference");
            assert!(
                registry.contains_key(&name),
                "protocol reference {} is absent from the parsed type registry",
                name.fullname(None)
            );
        }
        Value::Array(branches) => {
            for branch in branches {
                resolve_protocol_type(branch, registry);
            }
        }
        Value::Object(object) => match object.get("type") {
            Some(Value::String(kind)) if kind == "array" => {
                resolve_protocol_type(&object["items"], registry);
            }
            Some(Value::String(kind)) if kind == "map" => {
                resolve_protocol_type(&object["values"], registry);
            }
            Some(physical) => resolve_protocol_type(physical, registry),
            None => {
                if let Some(items) = object.get("items") {
                    resolve_protocol_type(items, registry);
                }
                if let Some(values) = object.get("values") {
                    resolve_protocol_type(values, registry);
                }
            }
        },
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

pub(crate) fn is_primitive(name: &str) -> bool {
    matches!(
        name,
        "null" | "boolean" | "int" | "long" | "float" | "double" | "bytes" | "string"
    )
}

pub(crate) fn collect_named_definitions(
    value: &Value,
    names: &mut std::collections::BTreeSet<String>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_named_definitions(value, names);
            }
        }
        Value::Object(object) => {
            if matches!(
                object.get("type").and_then(Value::as_str),
                Some("record" | "enum" | "fixed")
            ) {
                let name = object["name"].as_str().expect("named declaration name");
                let namespace = object
                    .get("namespace")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let full_name = if namespace.is_empty() {
                    name.to_owned()
                } else {
                    format!("{namespace}.{name}")
                };
                assert!(names.insert(full_name), "duplicate named declaration");
            }
            for nested in object.values() {
                collect_named_definitions(nested, names);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

pub(crate) fn artifact_pairs(artifacts: &[morphir_extension_sdk::Artifact]) -> Vec<(&str, &str)> {
    artifacts
        .iter()
        .map(|artifact| (artifact.path.as_str(), artifact.content.as_str()))
        .collect()
}

pub(crate) fn diagnostic_keys(
    diagnostics: &[morphir_extension_sdk::Diagnostic],
) -> Vec<(
    morphir_extension_sdk::DiagnosticSeverity,
    Option<&str>,
    &str,
)> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.severity,
                diagnostic.code.as_deref(),
                diagnostic.message.as_str(),
            )
        })
        .collect()
}
