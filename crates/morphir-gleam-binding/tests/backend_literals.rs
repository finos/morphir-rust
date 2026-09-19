//! Generated literals retain their value and their Gleam scalar kind.
use indexmap::IndexMap;
use morphir_core::ir::v4::*;
use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::{
    GleamExtension,
    frontend::{ast, parse_gleam},
};

fn generate(literal: Literal) -> GenerateResult {
    let (module_name, type_name) = match &literal {
        Literal::Bool(_) => ("basics", "bool"),
        Literal::Integer(_) => ("basics", "int"),
        Literal::Float(_) => ("basics", "float"),
        Literal::String(_) => ("string", "string"),
        Literal::Char(_) => ("char", "char"),
        Literal::Decimal(_) => ("decimal", "decimal"),
        Literal::Document(_) => ("document", "document"),
    };
    let output_type = Type::Reference(
        TypeAttributes::default(),
        morphir_core::naming::FQName {
            package_path: morphir_core::naming::PackageName::parse("morphir/SDK").into(),
            module_path: morphir_core::naming::ModuleName::parse(module_name).into(),
            local_name: morphir_core::naming::Name::from(type_name),
        },
        vec![],
    );
    let module = ModuleDefinition {
        doc: None,
        types: IndexMap::new(),
        values: IndexMap::from([(
            "sample".into(),
            AccessControlled {
                access: Access::Public,
                value: Documented::new(
                    None,
                    ValueDefinition {
                        input_types: IndexMap::new(),
                        output_type: Some(output_type),
                        body: ValueBody::Expression(Value::Literal(
                            ValueAttributes::default(),
                            literal,
                        )),
                    },
                ),
            },
        )]),
    };
    let package = PackageDefinition {
        modules: IndexMap::from([(
            "main".into(),
            AccessControlled {
                access: Access::Public,
                value: module,
            },
        )]),
    };
    GleamExtension
        .generate(GenerateRequest {
            target: "gleam".into(),
            ir: serde_json::to_value(package).unwrap(),
            options: Default::default(),
        })
        .unwrap()
}

fn parsed_literal(code: &str) -> ast::Literal {
    let module = parse_gleam("main.gleam", code).expect("generated source must parse");
    fn literal(expr: &ast::Expr) -> ast::Literal {
        match expr {
            ast::Expr::Literal { value } => value.clone(),
            ast::Expr::Block { statements } if statements.len() == 1 => {
                let ast::Statement::Expression(value) = &statements[0] else {
                    panic!("expected literal expression")
                };
                literal(value)
            }
            other => panic!("expected literal, got {other:?}"),
        }
    }
    literal(&module.values[0].body)
}

#[test]
fn strings_escape_gleam_syntax_without_changing_contents() {
    for value in [
        "quote: \"; slash: \\",
        "line\nreturn\rtab\t",
        "\0\u{8}\u{c}\u{1f}\u{7f}",
        "emoji 😀 and λ",
        "literal \\n and \\u{41}",
    ] {
        let result = generate(Literal::String(value.into()));
        assert!(result.success, "{:?}", result.diagnostics);
        assert_eq!(result.artifacts.len(), 1);
        match parsed_literal(&result.artifacts[0].content) {
            ast::Literal::String { value: actual } => assert_eq!(actual, value),
            other => panic!("expected String, got {other:?}"),
        }
    }
}

#[test]
fn floats_remain_floats_and_preserve_ieee_bits() {
    for value in [
        1.0,
        -0.0,
        0.0,
        -12.5,
        1e100,
        1e-100,
        f64::MIN_POSITIVE,
        f64::MAX,
    ] {
        let result = generate(Literal::Float(FloatLiteral::from_f64(value)));
        assert!(result.success, "{:?}", result.diagnostics);
        match parsed_literal(&result.artifacts[0].content) {
            ast::Literal::Float { value: actual } => assert_eq!(actual.to_bits(), value.to_bits()),
            other => panic!("expected Float for {value:?}, got {other:?}"),
        }
    }
}

#[test]
fn literals_without_exact_gleam_types_fail_without_artifacts() {
    for (literal, kind) in [
        (Literal::Char('λ'), "character"),
        (
            Literal::Decimal(morphir_core::ir::decimal::DecimalLiteral::parse("10.50").unwrap()),
            "decimal",
        ),
        (
            Literal::Document(serde_json::json!({"value": 1})),
            "document",
        ),
    ] {
        let result = generate(literal);
        assert!(!result.success, "must reject {kind}");
        assert!(result.artifacts.is_empty());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains(kind)),
            "{:?}",
            result.diagnostics
        );
    }
}

#[test]
#[ignore = "requires an installed Gleam compiler"]
fn generated_scalars_pass_the_gleam_compiler() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("gleam.toml"),
        "name = \"scalar_generation\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let values = [
        Literal::String("quote: \" slash: \\ newline:\n controls:\0\u{8}\u{c} emoji:😀".into()),
        Literal::Float(FloatLiteral::from_f64(1.0)),
        Literal::Float(FloatLiteral::from_f64(-0.0)),
        Literal::Float(FloatLiteral::from_f64(f64::MAX)),
        Literal::Float(FloatLiteral::from_f64(f64::MIN_POSITIVE)),
    ];
    for (index, value) in values.into_iter().enumerate() {
        let result = generate(value);
        assert!(result.success, "{:?}", result.diagnostics);
        std::fs::write(
            project.path().join(format!("src/sample_{index}.gleam")),
            &result.artifacts[0].content,
        )
        .unwrap();
    }
    let compiler = std::env::var_os("MORPHIR_TEST_GLEAM").unwrap_or_else(|| "gleam".into());
    let output = std::process::Command::new(compiler)
        .args(["check", "--target", "javascript"])
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
