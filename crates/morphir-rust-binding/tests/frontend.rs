use morphir_extension_sdk::{
    CompileOptions, CompilePackage, CompileRequest, CompileResult, Frontend, SourceDocument,
};
use morphir_rust_binding::RustExtension;
use serde_json::{Value, json};

fn request(source: &str, version: &str) -> CompileRequest {
    CompileRequest {
        language_id: "rust".into(),
        documents: vec![SourceDocument {
            uri: "file:///src/models.rs".into(),
            language_id: "rust".into(),
            text: source.into(),
            ..Default::default()
        }],
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: Some(vec!["Models".into()]),
        },
        options: CompileOptions {
            types_only: true,
            ir_version: version.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn compile(source: &str, version: &str) -> CompileResult {
    RustExtension.compile(request(source, version)).unwrap()
}
fn rejection(source: &str) {
    let result = compile(source, "3");
    assert!(!result.success, "accepted {source}");
    assert!(result.ir.is_none());
    assert!(result.modules.is_empty());
    assert!(
        result.diagnostics[0].location.is_some(),
        "missing location: {:?}",
        result.diagnostics
    );
}
#[test]
fn typed_records_custom_types_aliases_and_sdk_types_in_both_formats() {
    let source = "/// Customer model\n#[derive(Debug, Clone, PartialEq)]\npub struct Customer<T> { pub id: i64, pub name: String, pub active: bool, pub tag: char, pub score: f64, pub items: Vec<T>, pub choice: Option<T>, pub result: Result<T, String>, pub next: Box<Node> }\npub struct Node;\npub struct Id(pub i64);\npub enum Event<T> { Empty, One(T), Named { value: T } }\npub type Pair<T> = (T, String);";
    for version in ["3", "4", "3.0.0", "4.0.0"] {
        let result = compile(source, version);
        assert!(result.success, "{:?}", result.diagnostics);
        assert_eq!(
            result.ir_version.as_deref(),
            Some(if version.starts_with('3') { "3" } else { "4" })
        );
        assert_eq!(result.modules, vec!["Models"]);
        let ir = result.ir.unwrap();
        if version.starts_with('3') {
            let parsed: morphir_core::ir::classic::Distribution =
                serde_json::from_value(ir.clone()).unwrap();
            assert_eq!(parsed.format_version, 3);
            let types = &ir["distribution"][3]["modules"][0][1]["value"]["types"];
            assert_eq!(types[0][1]["value"]["doc"], "Customer model");
            assert_eq!(types[0][1]["value"]["value"][0], "TypeAliasDefinition");
            assert_eq!(types[1][1]["value"]["value"][0], "CustomTypeDefinition");
        } else {
            let _: morphir_core::ir::v4::IRFile = serde_json::from_value(ir).unwrap();
        }
    }
}
#[test]
fn rejects_unsupported_syntax_and_invalid_names_with_locations() {
    for source in [
        "pub struct S { pub x: &str }",
        "pub struct S<T: Clone> { pub x: T }",
        "pub struct S<'a> { pub x: &'a str }",
        "pub struct S<const N: usize>;",
        "pub trait X {}",
        "use std::string::String;",
        "mod x {}",
        "pub enum X { A = 1 }",
        "#[cfg(test)] pub struct X;",
        "#[derive(Custom)] pub struct X;",
        "macro_rules! x { () => {} }",
        "pub struct X { pub x: Unknown }",
        "pub struct X<T> { pub x: T } pub type Y = X;",
        "pub type X<T> = String;",
        "pub type X = Y; pub type Y = X;",
        "pub struct FooBar; pub struct Foo_Bar;",
        "pub struct X { pub foo_bar: i64, pub fooBar: i64 }",
        "pub struct X { pub a: i64, b: i64 }",
        "pub struct X; pub struct X;",
        "pub struct X {",
        "pub struct X<T, T> { pub x: T }",
    ] {
        rejection(source);
    }
}
#[test]
fn local_names_shadow_builtins_and_private_constructors_remain_private() {
    let result = compile(
        "pub struct String; pub type Local = String; pub struct Hidden(i64); struct Private { x: bool }",
        "3",
    );
    assert!(result.success, "{:?}", result.diagnostics);
    let ir = result.ir.unwrap();
    let types = &ir["distribution"][3]["modules"][0][1]["value"]["types"];
    assert_eq!(
        types[1][1]["value"]["value"][2][2][0],
        json!([["acme"], ["example"]])
    );
    assert_eq!(types[2][1]["value"]["value"][2]["access"], "Private");
    assert_eq!(types[3][1]["access"], "Private");
}
#[test]
fn types_only_skips_functions_with_warning_otherwise_compiles() {
    let mut req = request("pub struct X; pub fn f() {}", "3");
    let result = RustExtension.compile(req.clone()).unwrap();
    assert!(result.success);
    assert_eq!(result.diagnostics.len(), 1);
    req.options.types_only = false;
    let result = RustExtension.compile(req).unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert!(result.diagnostics.is_empty());
}
#[test]
fn validates_request_and_cli_context() {
    for version in ["2", "5", "3.1.0", "4.1.0", "junk"] {
        assert!(!compile("pub struct X;", version).success);
    }
    let mut req = request("pub struct X;", "3");
    req.options.extra = serde_json::from_value(json!({"outputDir":"out", "sourceRootUri":"file:///src", "sourceRoot":"src", "emitParseStage":false, "emitParseStageFatal":false})).unwrap();
    assert!(RustExtension.compile(req.clone()).unwrap().success);
    req.options
        .extra
        .insert("unknown".into(), Value::Bool(true));
    assert!(!RustExtension.compile(req.clone()).unwrap().success);
    req.options.extra.remove("unknown");
    req.options
        .extra
        .insert("outputDir".into(), Value::Bool(true));
    assert!(!RustExtension.compile(req.clone()).unwrap().success);
    req.options.extra.clear();
    req.documents.push(req.documents[0].clone());
    assert!(!RustExtension.compile(req).unwrap().success);
}
#[test]
fn source_locations_use_utf16_columns() {
    let source = "/* 🦀 */ pub struct X { pub x: &str }";
    let result = compile(source, "3");
    let location = result.diagnostics[0].location.as_ref().unwrap();
    assert_eq!(location.range.start.line, 0);
    assert_eq!(
        location.range.start.character as usize,
        source[..source.find('&').unwrap()].encode_utf16().count()
    );
}
#[test]
fn result_parameter_order_matches_the_sdk_contract() {
    let result = compile("pub type Answer = Result<String, bool>;", "3");
    assert!(result.success, "{:?}", result.diagnostics);
    let ir = result.ir.unwrap();
    let args =
        &ir["distribution"][3]["modules"][0][1]["value"]["types"][0][1]["value"]["value"][2][3];
    assert_eq!(args[0][2][2], json!(["bool"]));
    assert_eq!(args[1][2][2], json!(["string"]));
}
#[test]
fn sdk_references_do_not_create_false_alias_cycles() {
    assert!(compile("pub type Int = i64;", "3").success);
}
#[test]
fn constructor_names_are_unique_within_a_morphir_module() {
    rejection("pub enum First { Same } pub enum Second { Same }");
    rejection("pub struct First; pub enum Second { First }");
}
#[test]
fn request_rejects_languages_dependencies_invalid_packages_and_exposed_modules() {
    let mut req = request("pub struct X;", "3");
    req.language_id = "elm".into();
    assert!(!RustExtension.compile(req).unwrap().success);
    let mut req = request("pub struct X;", "3");
    req.documents[0].language_id = "elm".into();
    assert!(!RustExtension.compile(req).unwrap().success);
    let mut req = request("pub struct X;", "3");
    req.dependencies.push(Default::default());
    assert!(!RustExtension.compile(req).unwrap().success);
    for name in ["", "Acme/Example", "acme//example"] {
        let mut req = request("pub struct X;", "3");
        req.package.name = name.into();
        assert!(
            !RustExtension.compile(req).unwrap().success,
            "accepted {name}"
        );
    }
    let mut req = request("pub struct X;", "3");
    req.package.exposed_modules = Some(vec!["Unknown".into()]);
    assert!(!RustExtension.compile(req).unwrap().success);
}
#[test]
fn unexposed_modules_and_requested_parse_output_are_explicit() {
    let mut req = request("pub struct X;", "3");
    req.package.exposed_modules = Some(vec![]);
    let result = RustExtension.compile(req).unwrap();
    assert!(result.success);
    assert_eq!(
        result.ir.unwrap()["distribution"][3]["modules"][0][1]["access"],
        "Private"
    );
    let mut req = request("pub struct X;", "3");
    req.options
        .extra
        .insert("emitParseStage".into(), json!(true));
    let result = RustExtension.compile(req.clone()).unwrap();
    assert!(result.success);
    assert_eq!(
        result.diagnostics[0].code.as_deref(),
        Some("RS_PARSE_STAGE_UNAVAILABLE")
    );
    req.options
        .extra
        .insert("emitParseStageFatal".into(), json!(true));
    let result = RustExtension.compile(req).unwrap();
    assert!(!result.success);
    assert!(result.ir.is_none());
}
#[test]
fn compatible_patch_requests_emit_the_baseline_the_frontend_writes() {
    for version in ["3.0.9", "4.0.7"] {
        let result = compile("pub struct X;", version);
        assert!(result.success, "{:?}", result.diagnostics);
        assert_eq!(result.ir_version.as_deref(), Some(&version[..1]));
    }
}
#[test]
fn raw_identifiers_resolve_to_the_same_rust_type_name() {
    for source in [
        "pub struct r#Thing; pub type Alias = Thing;",
        "pub type Alias<T> = r#T;",
        "pub struct Thing; pub type Alias = r#Thing;",
    ] {
        let result = compile(source, "3");
        assert!(result.success, "{:?}", result.diagnostics);
    }
}
#[test]
fn module_file_names_must_have_a_representable_morphir_name() {
    for stem in ["__", "模型"] {
        let mut req = request("pub struct X;", "3");
        req.documents[0].uri = format!("file:///src/{stem}.rs");
        req.package.exposed_modules = Some(vec![]);
        assert!(!RustExtension.compile(req).unwrap().success);
    }
}

#[test]
fn recursive_named_structs_use_nominal_definitions() {
    let sources = [
        "pub struct Node { pub next: Option<Box<Node>> }",
        "pub struct First { pub next: Option<Box<Second>> } pub struct Second { pub next: Option<Box<First>> }",
        "pub struct Node { pub next: Option<Box<Link>> } pub type Link = Node;",
        "pub struct Node { pub next: Option<Box<Edge>> } pub enum Edge { Link(Node) }",
    ];
    for source in sources {
        for version in ["3", "4"] {
            let result = compile(source, version);
            assert!(result.success, "{source}: {:?}", result.diagnostics);
            if version == "3" {
                let ir = result.ir.unwrap();
                let types = &ir["distribution"][3]["modules"][0][1]["value"]["types"];
                assert_eq!(types[0][1]["value"]["value"][0], "CustomTypeDefinition");
                assert_eq!(types[0][1]["value"]["value"][2]["access"], "Public");
                if source.starts_with("pub struct First") {
                    assert_eq!(types[1][1]["value"]["value"][0], "CustomTypeDefinition");
                }
            }
        }
    }
}

#[test]
fn enum_fields_must_inherit_enum_visibility() {
    for source in [
        "pub enum E { V(pub i64) }",
        "pub enum E { V { pub value: i64 } }",
        "pub enum E { V(pub(crate) i64) }",
    ] {
        let result = compile(source, "3");
        assert!(!result.success, "accepted {source}");
        assert!(result.ir.is_none());
        assert_eq!(result.diagnostics[0].code.as_deref(), Some("RS_VISIBILITY"));
        let location = result.diagnostics[0].location.as_ref().unwrap();
        assert_eq!(
            location.range.start.character as usize,
            source.rfind("pub").unwrap()
        );
    }
}

#[test]
fn shared_alias_dependencies_are_visited_once() {
    let mut source = String::from("pub type A0 = i64;\n");
    for i in 1..=50 {
        source.push_str(&format!("pub type A{i} = (A{}, A{});\n", i - 1, i - 1));
    }
    assert!(compile(&source, "3").success);
}
