//! Sum types retain their discriminants, payloads and recursion across both IR releases.
use morphir_core::ir::{classic, v4};
use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::GleamExtension;

const SOURCE: &str = r#"
pub type Outcome(a) {
  Pending
  Succeeded(value: a)
  Failed(message: String, code: Int)
}

pub type Tree(a) {
  Leaf(a)
  Branch(left: Tree(a), right: Tree(a))
}
"#;

fn compile(version: &str, source: &str) -> CompileResult {
    let result = GleamExtension
        .compile(CompileRequest {
            language_id: "gleam".into(),
            sources: SourceSet {
                root: None,
                documents: vec![SourceDocument {
                    uri: "file:///src/unions.gleam".into(),
                    language_id: "gleam".into(),
                    version: 1,
                    text: source.into(),
                }],
            },
            package: CompilePackage {
                name: "example/unions".into(),
                exposed_modules: None,
            },
            options: CompileOptions {
                ir_version: version.into(),
                types_only: true,
                extra: [("emitParseStage".into(), false.into())].into(),
            },
            ..Default::default()
        })
        .unwrap();
    assert!(result.success, "{version}: {:?}", result.diagnostics);
    result
}

fn module(result: &CompileResult) -> v4::ModuleDefinition {
    let ir = result.ir.clone().unwrap();
    let ir: v4::IRFile = if ir["formatVersion"] == 3 {
        let classic: classic::Distribution = serde_json::from_value(ir).unwrap();
        morphir_core::migration::migrate_distribution(&classic, Default::default())
            .unwrap()
            .value
    } else {
        serde_json::from_value(ir).unwrap()
    };
    let v4::Distribution::Library(library) = ir.distribution else {
        panic!("expected a library")
    };
    library.def.modules["unions"].value.clone()
}

fn generate(result: CompileResult) -> String {
    let generated = GleamExtension
        .generate(GenerateRequest {
            ir: result.ir.unwrap(),
            target: "gleam".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert_eq!(generated.artifacts.len(), 1);
    generated.artifacts[0].content.clone()
}

#[test]
fn discriminated_unions_preserve_variants_payloads_and_recursion_in_v3_and_v4() {
    for version in ["3", "4"] {
        let compiled = compile(version, SOURCE);
        let original = module(&compiled);
        let v4::TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } = &original.types["outcome"].value.value
        else {
            panic!("expected a sum type")
        };
        assert_eq!(type_params, &[morphir_core::naming::Name::from("a")]);
        assert_eq!(constructors.access, v4::Access::Public);
        assert_eq!(
            constructors
                .value
                .iter()
                .map(|c| c.name.to_string())
                .collect::<Vec<_>>(),
            ["pending", "succeeded", "failed"]
        );
        assert!(constructors.value[0].args.is_empty());
        assert_eq!(constructors.value[1].args[0].name.to_string(), "value");
        assert!(
            matches!(&constructors.value[1].args[0].arg_type, v4::Type::Variable(_, name) if name.to_string() == "a")
        );
        let failed = &constructors.value[2].args;
        assert_eq!(
            failed
                .iter()
                .map(|a| a.name.to_string())
                .collect::<Vec<_>>(),
            ["message", "code"]
        );
        for (arg, expected) in failed
            .iter()
            .zip(["morphir/SDK:string#string", "morphir/SDK:basics#int"])
        {
            assert!(
                matches!(&arg.arg_type, v4::Type::Reference(_, name, _) if name.to_string() == expected)
            );
        }
        let v4::TypeDefinition::CustomTypeDefinition { constructors, .. } =
            &original.types["tree"].value.value
        else {
            panic!("expected a recursive sum type")
        };
        assert_eq!(constructors.value.len(), 2);
        for arg in &constructors.value[1].args {
            assert!(
                matches!(&arg.arg_type, v4::Type::Reference(_, name, params) if name.to_string() == "example/unions:unions#tree" && matches!(&params[..], [v4::Type::Variable(_, a)] if a.to_string() == "a"))
            );
        }
        let generated = generate(compiled);
        assert_eq!(
            module(&compile(version, &generated)).types,
            original.types,
            "{version}: {generated}"
        );
    }
}

#[test]
#[ignore = "requires an installed Gleam compiler"]
fn generated_sum_types_pass_the_gleam_compiler_for_both_ir_versions() {
    for version in ["3", "4"] {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("src")).unwrap();
        std::fs::write(
            project.path().join("gleam.toml"),
            "name = \"sum_types\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            project.path().join("src/unions.gleam"),
            generate(compile(version, SOURCE)),
        )
        .unwrap();
        let compiler = std::env::var_os("MORPHIR_TEST_GLEAM").unwrap_or_else(|| "gleam".into());
        let output = std::process::Command::new(compiler)
            .args(["check", "--target", "javascript"])
            .current_dir(project.path())
            .output()
            .expect("run Gleam compiler");
        assert!(
            output.status.success(),
            "{version}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
