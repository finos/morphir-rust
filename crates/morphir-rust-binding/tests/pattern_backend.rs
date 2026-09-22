use morphir_core::{ir::v4::*, naming::Name};
use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_rust_binding::RustExtension;
use serde_json::json;
fn variable(name: &str) -> Value {
    Value::Variable(
        Default::default(),
        Name::from_canonical_string(name).unwrap(),
    )
}
fn boolean(value: bool) -> Pattern {
    Pattern::LiteralPattern(Default::default(), Literal::Bool(value))
}
fn integer(value: i64) -> Value {
    Value::Literal(Default::default(), Literal::Integer(value.into()))
}
fn generate(cases: Vec<(Pattern, Value)>) -> morphir_extension_sdk::GenerateResult {
    let definition = ValueDefinition {
        input_types: [(
            "value".into(),
            serde_json::from_value(json!("morphir/SDK:basics#bool")).unwrap(),
        )]
        .into_iter()
        .collect(),
        output_type: Some(serde_json::from_value(json!("morphir/SDK:basics#int")).unwrap()),
        body: ValueBody::Expression(Value::PatternMatch(
            Default::default(),
            Box::new(variable("value")),
            cases.into_iter().map(|(p, v)| PatternCase(p, v)).collect(),
        )),
    };
    let ir = json!({"formatVersion":4,"distribution":{"Library":{"packageName":"acme/example","dependencies":{},"def":{"modules":{"models":{"Public":{"types":{},"values":{"select":{"Public":serde_json::to_value(definition).unwrap()}}}}}}}}});
    RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options: Default::default(),
        })
        .unwrap()
}
#[test]
fn direct_ir_match_emits_ordered_exhaustive_rust() {
    let result = generate(vec![
        (boolean(true), integer(1)),
        (boolean(false), integer(2)),
    ]);
    assert!(result.success, "{:?}", result.diagnostics);
    let source = &result.artifacts[0].content;
    assert!(source.contains("match value"), "{source}");
    assert!(!source.contains("panic!"));
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("main.rs");
    let output = dir.path().join("consumer");
    std::fs::write(&input,format!("{source}\nfn main() {{ assert_eq!(models::select(true),1); assert_eq!(models::select(false),2); }}")).unwrap();
    let compile = std::process::Command::new("rustc")
        .args(["--edition=2024"])
        .arg(input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(
        std::process::Command::new(output)
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn malformed_direct_ir_is_rejected_without_artifacts() {
    for cases in [
        vec![],
        vec![(boolean(true), integer(1))],
        vec![
            (boolean(true), integer(1)),
            (boolean(false), variable("missing")),
        ],
        vec![(
            Pattern::WildcardPattern(Default::default()),
            Value::Unit(Default::default()),
        )],
    ] {
        let result = generate(cases);
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
    }
}

fn custom_ir(access: Access, payload: Option<Type>, phantom: bool) -> serde_json::Value {
    use morphir_core::naming::FQName;
    let name = |text| Name::from_canonical_string(text).unwrap();
    let fq = |text| FQName::from_canonical_string(text).unwrap();
    let subject = Type::Reference(
        Default::default(),
        fq("acme/example:models#choice"),
        if phantom {
            vec![Type::Variable(Default::default(), name("a"))]
        } else {
            vec![]
        },
    );
    let constructor = ConstructorDefinition {
        name: name("chosen"),
        args: payload
            .into_iter()
            .map(|arg_type| ConstructorArg {
                name: name("payload"),
                arg_type,
            })
            .collect(),
    };
    let pattern = Pattern::ConstructorPattern(
        Default::default(),
        fq("acme/example:models#chosen"),
        constructor
            .args
            .iter()
            .map(|_| Pattern::WildcardPattern(Default::default()))
            .collect(),
    );
    let definition = ValueDefinition {
        input_types: [("value".into(), subject)].into_iter().collect(),
        output_type: Some(Type::Unit(Default::default())),
        body: ValueBody::Expression(Value::PatternMatch(
            Default::default(),
            Box::new(variable("value")),
            vec![PatternCase(pattern, Value::Unit(Default::default()))],
        )),
    };
    let custom = TypeDefinition::CustomTypeDefinition {
        type_params: if phantom { vec![name("a")] } else { vec![] },
        constructors: AccessControlled {
            access,
            value: vec![constructor],
        },
    };
    json!({"formatVersion":4,"distribution":{"Library":{"packageName":"acme/example","dependencies":{},"def":{"modules":{"models":{"Public":{"types":{"choice":{"Public":custom}},"values":{"select":{"Public":definition}}}}}}}}})
}
fn generate_ir(ir: serde_json::Value) -> morphir_extension_sdk::GenerateResult {
    RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options: Default::default(),
        })
        .unwrap()
}
#[test]
fn phantom_constructor_patterns_account_for_generated_marker_fields() {
    let result = generate_ir(custom_ir(Access::Public, None, true));
    assert!(result.success, "{:?}", result.diagnostics);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("lib.rs");
    std::fs::write(&input, &result.artifacts[0].content).unwrap();
    let output = std::process::Command::new("rustc")
        .args(["--edition=2024", "--crate-type=lib"])
        .arg(&input)
        .arg("--out-dir")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn unsupported_constructor_representations_are_diagnosed() {
    let recursive: Type = serde_json::from_value(json!("acme/example:models#choice")).unwrap();
    for ir in [
        custom_ir(Access::Private, None, false),
        custom_ir(Access::Public, Some(recursive), false),
        custom_ir(
            Access::Public,
            Some(Type::Record(Default::default(), vec![])),
            false,
        ),
    ] {
        let result = generate_ir(ir);
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
    }
}

fn execute(source: &str, consumer: &str) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("main.rs");
    let output = dir.path().join("consumer");
    std::fs::write(&input, format!("{source}\nfn main() {{ {consumer} }}")).unwrap();
    let compile = std::process::Command::new("rustc")
        .args(["--edition=2024"])
        .arg(input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{source}\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(
        std::process::Command::new(output)
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn named_field_patterns_execute_after_flattening_in_both_versions() {
    use morphir_extension_sdk::{
        CompileOptions, CompilePackage, CompileRequest, Frontend, SourceDocument, SourceSet,
    };
    for version in ["3", "4"] {
        let compiled=RustExtension.compile(CompileRequest {
            language_id:"rust".into(),sources:SourceSet { root: None, documents:vec![SourceDocument { uri:"models.rs".into(),language_id:"rust".into(),text:"pub enum Choice { Full { left: i64, right: bool }, Empty } pub fn select(x: Choice) -> i64 { match x { Choice::Full { right: true, left } => left, Choice::Full { left, .. } => left, Choice::Empty => 0 } }".into(),..Default::default() }] },
            package:CompilePackage { name:"acme/example".into(), exposed_modules:Some(vec!["Models".into()]) },options:CompileOptions { types_only:false,ir_version:version.into(),..Default::default() },..Default::default()
        }).unwrap();
        assert!(compiled.success, "{:?}", compiled.diagnostics);
        let generated = generate_ir(compiled.ir.unwrap());
        assert!(generated.success, "{:?}", generated.diagnostics);
        execute(
            &generated.artifacts[0].content,
            "assert_eq!(models::select(models::Choice::Full(42,true)),42); assert_eq!(models::select(models::Choice::Full(21,false)),21); assert_eq!(models::select(models::Choice::Empty),0);",
        );
    }
}
#[test]
fn direct_ir_pattern_shadowing_does_not_capture_outer_binding() {
    let mut ir = custom_ir(Access::Public, None, false);
    let name = Name::from_canonical_string("value").unwrap();
    let int: Type = serde_json::from_value(json!("morphir/SDK:basics#int")).unwrap();
    let inner = Value::PatternMatch(
        Default::default(),
        Box::new(integer(9)),
        vec![PatternCase(
            Pattern::AsPattern(
                Default::default(),
                Box::new(Pattern::WildcardPattern(Default::default())),
                name,
            ),
            variable("value"),
        )],
    );
    let definition = ValueDefinition {
        input_types: [("value".into(), int.clone())].into_iter().collect(),
        output_type: Some(Type::Tuple(Default::default(), vec![int.clone(), int])),
        body: ValueBody::Expression(Value::Tuple(
            Default::default(),
            vec![inner, variable("value")],
        )),
    };
    ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"]["select"]["Public"] =
        serde_json::to_value(definition).unwrap();
    let generated = generate_ir(ir);
    assert!(generated.success, "{:?}", generated.diagnostics);
    execute(
        &generated.artifacts[0].content,
        "assert_eq!(models::select(42),(9,42));",
    );
}
