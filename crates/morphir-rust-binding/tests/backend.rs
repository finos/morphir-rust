use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_rust_binding::RustExtension;
use serde_json::{Value, json};

fn alias(params: &[&str], tpe: Value) -> Value {
    json!({"Public":{"TypeAliasDefinition":{"typeParams":params,"typeExp":tpe}}})
}

fn library(types: Value) -> Value {
    json!({"formatVersion":4,"distribution":{"Library":{"packageName":"acme/example","dependencies":{},"def":{"modules":{"models":{"Public":{"types":types,"values":{}}}}}}}})
}

fn generate(ir: Value) -> String {
    let result = RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts.len(), 1);
    assert_eq!(result.artifacts[0].path, "lib.rs");
    result.artifacts[0].content.clone()
}

fn check(source: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    let result = std::process::Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "--crate-name=generated",
        ])
        .arg(&path)
        .arg("--out-dir")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn generates_all_seven_type_forms_and_recursive_generic_adts() {
    let ir = library(json!({
        "identity": alias(&["a"], json!("a")),
        "count": alias(&[], json!("morphir/SDK:basics#int")),
        "pair": alias(&["a"], json!({"Tuple":["a","morphir/SDK:string#string"]})),
        "person": alias(&[], json!({"Record":{"fields":{"name":"morphir/SDK:string#string","age":"morphir/SDK:basics#int"}}})),
        "extended": alias(&["r"], json!({"ExtensibleRecord":{"variable":"r","fields":{"id":"morphir/SDK:basics#int"}}})),
        "handler": alias(&["a"], json!({"Function":{"parameterType":"a","returnType":{"Unit":{}}}})),
        "nothing": alias(&[], json!({"Unit":{}})),
        "tree":{"Public":{"CustomTypeDefinition":{"typeParams":["a"],"access":"Public","constructors":{
            "leaf":[["value","a"]],
            "branch":[["left",{"Reference":["acme/example:models#tree","a"]}],["right",{"Reference":["acme/example:models#tree","a"]}]]
        }}}},
        "phantom":{"Public":{"CustomTypeDefinition":{"typeParams":["a"],"access":"Public","constructors":{"empty":[]}}}},
        "uninhabited":{"Public":{"CustomTypeDefinition":{"typeParams":["a"],"access":"Public","constructors":{}}}},
        "nested": alias(&["a"], json!({"Tuple":[{"Record":{"fields":{"data":"a"}}},{"Reference":["morphir/SDK:maybe#maybe","a"]}]}))
    }));
    let source = generate(ir);
    assert!(source.contains("::std::ops::Fn"));
    assert!(source.contains("remaining_fields"));
    assert!(source.contains("enum Tree"));
    check(&format!(
        "{source}\nfn use_types() {{\n let _: models::Tree<i64> = models::Tree::Leaf(1);\n let _: models::Extended<()> = models::Extended {{ id: 1, remaining_fields: () }};\n let callback: models::Handler<i64> = std::rc::Rc::new(|_| ()); callback(1);\n}}"
    ));
}

#[test]
fn v3_and_v4_generate_identical_types() {
    let v3 = json!({"formatVersion":3,"distribution":["Library",[["acme"],["example"]],[],{"modules":[[[["models"]],{"access":"Public","value":{"types":[[["count"],{"access":"Public","value":{"doc":"","value":["TypeAliasDefinition",[],["Reference",{},[[["morphir"],["s","d","k"]],[["basics"]],["int"]],[]]]}}]],"values":[]}}]]}]});
    assert_eq!(
        generate(v3),
        generate(library(
            json!({"count": alias(&[], json!("morphir/SDK:basics#int"))})
        ))
    );
}

#[test]
fn refuses_invalid_types_versions_options_and_incomplete_definitions() {
    for ir in [
        library(json!({"bad":alias(&[], json!("a"))})),
        library(json!({"bad":alias(&[], json!("acme/example:models#missing"))})),
        library(json!({"bad":alias(&[], json!({"Reference":["morphir/SDK:list#list"]}))})),
        library(json!({"bad":alias(&[], json!("acme/example:models#bad"))})),
        library(
            json!({"bad":{"Public":{"IncompleteTypeDefinition":{"typeParams":[],"incompleteness":{"Draft":{}}}}}}),
        ),
        json!({"formatVersion":"4.1.0","distribution":{}}),
        json!({"formatVersion":5,"distribution":{}}),
    ] {
        let result = RustExtension
            .generate(GenerateRequest {
                ir: ir.clone(),
                target: "rust".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(!result.success, "accepted {ir}");
        assert!(result.artifacts.is_empty());
        assert!(!result.diagnostics.is_empty());
    }
    let result = RustExtension
        .generate(GenerateRequest {
            ir: library(json!({})),
            target: "rust".into(),
            options: [("unknown".into(), json!(true))].into(),
        })
        .unwrap();
    assert!(!result.success);
}

#[test]
fn specs_support_derived_types_and_explicit_opaque_bindings() {
    let ir = json!({"formatVersion":4,"distribution":{"Specs":{"packageName":"acme/example","dependencies":{},"spec":{"modules":{"models":{"types":{
        "secret":{"OpaqueTypeSpecification":{"typeParams":[]}},
        "positive":{"DerivedTypeSpecification":{"typeParams":[],"baseType":"morphir/SDK:basics#int","fromBaseType":"acme/example:models#from-int","toBaseType":"acme/example:models#to-int"}}
    },"values":{}}}}}}});
    let options = [(
        "externalTypes".into(),
        json!({"acme/example:models#secret":"std::string::String"}),
    )]
    .into();
    let result = RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options,
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    let source = &result.artifacts[0].content;
    assert!(source.contains("from-int"));
    assert!(source.contains("struct Positive"));
    check(source);
}

#[test]
fn sdk_result_order_collections_keywords_and_private_constructors_compile() {
    let ir = library(json!({
        "response":alias(&[],json!({"Reference":["morphir/SDK:result#result","morphir/SDK:string#string","morphir/SDK:basics#int"]})),
        "table":alias(&[],json!({"Reference":["morphir/SDK:dict#dict","morphir/SDK:string#string","morphir/SDK:basics#int"]})),
        "tags":alias(&[],json!({"Reference":["morphir/SDK:set#set","morphir/SDK:string#string"]})),
        "keyword":alias(&[],json!({"Record":{"fields":{"type":"morphir/SDK:basics#bool"}}})),
        "secret":{"Public":{"CustomTypeDefinition":{"typeParams":["a"],"access":"Private","constructors":{"secret":[["data","a"]]}}}},
        "hidden":{"Private":{"TypeAliasDefinition":{"typeParams":[],"typeExp":"morphir/SDK:basics#int"}}}
    }));
    let source = generate(ir);
    assert!(source.contains("pub(crate) type Hidden"));
    assert!(source.contains("pub struct Secret"));
    check(&format!(
        "{source}\nfn use_types() {{ let _: models::Response = Ok(42); let _: models::Response = Err(String::new()); let _ = models::Keyword {{ r#type: true }}; }}"
    ));
}

#[test]
fn v3_full_vocabulary_survives_typed_migration_and_generation() {
    let unit = json!(["Unit", {}]);
    let variable = json!(["Variable", {}, ["a"]]);
    let reference = json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["maybe"]], ["maybe"]],
        [variable.clone()]
    ]);
    let forms = [
        ("unit", unit.clone(), vec![]),
        ("variable", variable.clone(), vec![json!(["a"])]),
        ("reference", reference, vec![json!(["a"])]),
        (
            "tuple",
            json!(["Tuple", {}, [unit.clone(), variable.clone()]]),
            vec![json!(["a"])],
        ),
        (
            "record",
            json!(["Record",{},[{"name":["field"],"tpe":variable.clone()}]]),
            vec![json!(["a"])],
        ),
        (
            "row",
            json!(["ExtensibleRecord",{},["r"],[{"name":["field"],"tpe":variable.clone()}]]),
            vec![json!(["r"]), json!(["a"])],
        ),
        (
            "function",
            json!(["Function", {}, variable, unit]),
            vec![json!(["a"])],
        ),
    ];
    let types: Vec<_> = forms.into_iter().map(|(name,tpe,params)| json!([[name],{"access":"Public","value":{"doc":"","value":["TypeAliasDefinition",params,tpe]}}])).collect();
    let v3 = json!({"formatVersion":3,"distribution":["Library",[["acme"],["example"]],[],{"modules":[[[["models"]],{"access":"Public","value":{"types":types,"values":[]}}]]}]});
    let typed: morphir_core::ir::classic::Distribution =
        serde_json::from_value(v3.clone()).unwrap();
    let v4 = morphir_core::migration::migrate_distribution(&typed, Default::default())
        .unwrap()
        .value;
    let source = generate(v3);
    assert_eq!(source, generate(serde_json::to_value(v4).unwrap()));
    check(&source);
}

#[test]
fn nested_modules_and_dependency_generic_bindings_resolve() {
    let mut ir = library(
        json!({"box":alias(&["a"],json!({"Reference":["vendor/types:containers#box","a"]}))}),
    );
    ir["distribution"]["Library"]["dependencies"] = json!({"vendor/types":{"modules":{"containers":{"types":{"box":{"OpaqueTypeSpecification":{"typeParams":["a"]}}},"values":{}}}}});
    ir["distribution"]["Library"]["def"]["modules"]["nested/consumer"] = json!({"Private":{"types":{"user":alias(&[],json!({"Reference":["acme/example:models#box","morphir/SDK:basics#int"]}))},"values":{}}});
    let result = RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options: [(
                "externalTypes".into(),
                json!({"vendor/types:containers#box":"std::boxed::Box"}),
            )]
            .into(),
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    check(&result.artifacts[0].content);
}

#[test]
fn opaque_bindings_and_name_collisions_fail_with_specific_diagnostics() {
    let cases = [
        (library(json!({"bad":alias(&[],json!("a"))})), "RS_TYPE"),
        (
            library(json!({"bad":alias(&[],json!("acme/example:models#absent"))})),
            "RS_REFERENCE",
        ),
        (
            library(
                json!({"bad":{"Public":{"IncompleteTypeDefinition":{"typeParams":[],"incompleteness":{"Draft":{}}}}}}),
            ),
            "RS_INCOMPLETE",
        ),
        (
            json!({"formatVersion":4,"distribution":{"Specs":{"packageName":"acme/example","dependencies":{},"spec":{"modules":{"models":{"types":{"secret":{"OpaqueTypeSpecification":{"typeParams":[]}}},"values":{}}}}}}}),
            "RS_OPAQUE",
        ),
    ];
    for (ir, code) in cases {
        let result = RustExtension
            .generate(GenerateRequest {
                ir,
                target: "rust".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(!result.success);
        assert_eq!(result.diagnostics[0].code.as_deref(), Some(code));
    }
}

#[test]
fn generated_function_trait_is_not_shadowed_by_rust_type_parameters() {
    check(&generate(library(
        json!({"handler":alias(&["fn"],json!({"Function":{"parameterType":"fn","returnType":{"Unit":{}}}}))}),
    )));
}

#[test]
fn empty_module_paths_generate_at_the_crate_root() {
    let mut ir = library(json!({"unit":alias(&[],json!({"Unit":{}}))}));
    let modules = ir["distribution"]["Library"]["def"]["modules"]
        .as_object_mut()
        .unwrap();
    let module = modules.remove("models").unwrap();
    modules.insert(String::new(), module);
    check(&generate(ir));
}

#[test]
fn ir_aliases_can_ignore_parameters_without_changing_the_aliased_type() {
    let source = generate(library(
        json!({"ignored":alias(&["a"],json!("morphir/SDK:basics#int"))}),
    ));
    check(&format!(
        "{source}\nfn use_alias() {{ let _: models::Ignored<String> = 42_i64; }}"
    ));
}

#[test]
fn distinct_morphir_names_cannot_collapse_into_one_rust_field_or_module() {
    let field_collision = library(
        json!({"collision":alias(&[],json!({"Record":{"fields":{"foo-bar":{"Unit":{}},"foo-BAR":{"Unit":{}}}}}))}),
    );
    let mut module_collision = library(json!({}));
    let modules = module_collision["distribution"]["Library"]["def"]["modules"]
        .as_object_mut()
        .unwrap();
    let module = modules.remove("models").unwrap();
    modules.insert("foo-bar".into(), module.clone());
    modules.insert("foo-BAR".into(), module);
    for ir in [field_collision, module_collision] {
        let result = RustExtension
            .generate(GenerateRequest {
                ir,
                target: "rust".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(!result.success);
        assert_eq!(result.diagnostics[0].code.as_deref(), Some("RS_NAME"));
    }
}
