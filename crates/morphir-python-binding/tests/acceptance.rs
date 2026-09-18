use cucumber::{World, given, then, when};
use morphir_extension_sdk::prelude::*;
use morphir_python_binding::PythonExtension;

#[derive(Debug, Default)]
struct TestDriver {
    source: String,
    ir_version: Option<String>,
    additional: Vec<SourceDocument>,
    compiled: Option<CompileResult>,
    generated: Option<GenerateResult>,
}

impl TestDriver {
    fn compile(&self, text: String) -> CompileResult {
        let mut documents = vec![SourceDocument {
            uri: "models.py".into(),
            language_id: "python".into(),
            version: 1,
            text,
        }];
        documents.extend(self.additional.clone());
        self.compile_documents(documents)
    }

    fn compile_documents(&self, documents: Vec<SourceDocument>) -> CompileResult {
        PythonExtension
            .compile(CompileRequest {
                language_id: "python".into(),
                documents,
                package: CompilePackage {
                    name: "acme/example".into(),
                    exposed_modules: None,
                },
                dependencies: vec![],
                options: CompileOptions {
                    ir_version: self.ir_version.clone().unwrap_or_else(|| "4".into()),
                    types_only: false,
                    ..Default::default()
                },
                baseline: None,
            })
            .unwrap()
    }

    fn compile_and_generate(&mut self) {
        let compiled = self.compile(self.source.clone());
        assert!(compiled.success, "{:?}", compiled.diagnostics);
        let generated = PythonExtension
            .generate(GenerateRequest {
                ir: compiled.ir.clone().unwrap(),
                target: "python".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(generated.success, "{:?}", generated.diagnostics);
        self.compiled = Some(compiled);
        self.generated = Some(generated);
    }

    fn assert_roundtrip(&self) {
        let recompiled = self.compile_documents(
            self.generated
                .as_ref()
                .unwrap()
                .artifacts
                .iter()
                .map(|a| SourceDocument {
                    uri: a.path.clone(),
                    language_id: "python".into(),
                    version: 1,
                    text: a.content.clone(),
                })
                .collect(),
        );
        assert!(recompiled.success, "{:?}", recompiled.diagnostics);
        assert_eq!(recompiled.ir, self.compiled.as_ref().unwrap().ir);
    }

    fn assert_rejected(&self) {
        let result = self.compiled.as_ref().unwrap();
        assert!(!result.success);
        assert!(result.ir.is_none());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Error)
        );
    }
}

#[derive(Debug, Default, World)]
struct PythonWorld {
    driver: TestDriver,
}

#[given(expr = "IR version {word}")]
fn ir_version(world: &mut PythonWorld, version: String) {
    world.driver.ir_version = Some(version);
}

#[given("a Python model with a product and a sum with payloads")]
fn model(world: &mut PythonWorld) {
    world.driver.source = include_str!("fixtures/models.py").into();
}

#[given("a Python model containing an unannotated function")]
fn function(world: &mut PythonWorld) {
    world.driver.source = "def value():\n    return 1\n".into();
}

#[given("annotated Python functions with conditional bodies")]
fn conditionals(world: &mut PythonWorld) {
    world.driver.source = include_str!("fixtures/conditionals.py").into();
}

#[given("Python tuple aliases and tuple-valued functions")]
fn tuples(world: &mut PythonWorld) {
    world.driver.source = include_str!("fixtures/tuples.py").into();
}

#[given("Python modules with imported ADTs and tuple aliases")]
fn modules(world: &mut PythonWorld) {
    world.driver.source = format!(
        "{}\n{}",
        include_str!("fixtures/models.py"),
        include_str!("fixtures/tuples.py")
    );
    world.driver.additional.push(SourceDocument {
        uri: "rules.py".into(),
        language_id: "python".into(),
        version: 1,
        text: include_str!("fixtures/modules/rules.py").into(),
    });
}
#[when("I compile the model and generate Python")]
fn roundtrip(world: &mut PythonWorld) {
    world.driver.compile_and_generate();
}

#[when("I compile the model")]
fn compile(world: &mut PythonWorld) {
    world.driver.compiled = Some(world.driver.compile(world.driver.source.clone()));
}

#[then("compiling the generated Python preserves the model")]
fn preserved(world: &mut PythonWorld) {
    world.driver.assert_roundtrip();
}

#[then("compilation fails without partial IR")]
fn rejected(world: &mut PythonWorld) {
    world.driver.assert_rejected();
}

#[tokio::main]
async fn main() {
    PythonWorld::cucumber()
        .fail_on_skipped()
        .run_and_exit(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/features"))
        .await;
}
