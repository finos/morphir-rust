use cucumber::{World, given, then, when};
use morphir_extension_sdk::prelude::*;
use morphir_rust_binding::RustExtension;

#[derive(Debug, Default)]
struct TestDriver {
    source: String,
    compiled: Option<CompileResult>,
    generated: Option<GenerateResult>,
}

impl TestDriver {
    fn compile(&mut self, version: &str) {
        self.compile_with_options(CompileOptions {
            types_only: true,
            ir_version: version.into(),
            ..Default::default()
        });
    }

    fn compile_with_options(&mut self, options: CompileOptions) {
        let version = options.ir_version.clone();
        let extension = NativeExtension::frontend_backend(RustExtension).unwrap();
        self.compiled = Some(
            extension
                .frontend()
                .unwrap()
                .compile(CompileRequest {
                    language_id: "rust".into(),
                    documents: vec![SourceDocument {
                        uri: "models.rs".into(),
                        language_id: "rust".into(),
                        version: 1,
                        text: self.source.clone(),
                    }],
                    package: CompilePackage {
                        name: "acme/example".into(),
                        exposed_modules: vec!["Models".into()],
                    },
                    dependencies: vec![],
                    options,
                })
                .unwrap(),
        );
        let compiled = self.compiled.as_ref().unwrap();
        if compiled.success {
            assert_eq!(compiled.ir_version.as_deref(), Some(version.as_str()));
            assert_eq!(
                compiled.ir.as_ref().unwrap()["formatVersion"],
                version.parse::<u64>().unwrap()
            );
        }
    }

    fn generate(&mut self) {
        let compiled = self.compiled.as_ref().expect("compile the model first");
        assert!(compiled.success, "{:?}", compiled.diagnostics);
        let extension = NativeExtension::frontend_backend(RustExtension).unwrap();
        let generated = extension
            .backend()
            .unwrap()
            .generate(GenerateRequest {
                ir: compiled.ir.clone().unwrap(),
                target: "rust".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(generated.success, "{:?}", generated.diagnostics);
        self.generated = Some(generated);
    }

    fn assert_usable_rust(&self) {
        let generated = self.generated.as_ref().expect("generate Rust first");
        assert_eq!(generated.artifacts.len(), 1);
        let artifact = &generated.artifacts[0];
        assert_eq!(artifact.path, "lib.rs");
        assert!(!artifact.binary);
        let consumer = r#"
fn main() {
    let person = models::Person { name: String::from("Ada"), age: 37 };
    let decision = models::Decision::Accepted(person);
    match decision {
        models::Decision::Accepted(person) => {
            assert_eq!(person.name, "Ada");
            assert_eq!(person.age, 37);
        }
        models::Decision::Pending => panic!("expected the accepted payload"),
    }
    let _pending = models::Decision::Pending;
}
"#;
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("main.rs");
        let executable = directory
            .path()
            .join(format!("consumer{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&source, format!("{}\n{consumer}", artifact.content)).unwrap();
        let compiled = std::process::Command::new("rustc")
            .args(["--edition=2024", "--crate-name=generated_consumer"])
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("rustc must be available for acceptance tests");
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let executed = std::process::Command::new(executable).output().unwrap();
        assert!(
            executed.status.success(),
            "{}",
            String::from_utf8_lossy(&executed.stderr)
        );
    }

    fn assert_bindings_preserved(&self) {
        let compiled = self.compiled.as_ref().unwrap();
        assert!(compiled.success, "{:?}", compiled.diagnostics);
        let ir = compiled.ir.as_ref().unwrap();
        let _: morphir_core::ir::v4::IRFile = serde_json::from_value(ir.clone()).unwrap();
        let values = &ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"];
        assert_eq!(
            values["add"]["Public"]["NativeBody"]["nativeInfo"]["hint"],
            serde_json::json!({"Arithmetic": {}})
        );
        assert_eq!(
            values["add"]["Public"]["NativeBody"]["inputTypes"]
                .as_object()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            values["identity"]["Public"]["ExternalBody"]["externals"][0]["externalName"],
            "vendor::identity"
        );
        assert_eq!(
            values["identity"]["Public"]["ExternalBody"]["outputType"],
            serde_json::json!({"Variable": {"name": "t"}})
        );
    }

    fn assert_rejected(&self) {
        let compiled = self.compiled.as_ref().expect("compile the model first");
        assert!(!compiled.success);
        assert!(compiled.ir.is_none());
        assert!(compiled.modules.is_empty());
        assert!(
            compiled
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        );
    }
}

#[derive(Debug, Default, World)]
struct RustWorld {
    driver: TestDriver,
}

#[given("a Rust model with a product and a sum with payloads")]
fn model(world: &mut RustWorld) {
    world.driver.source = "pub struct Person { pub name: String, pub age: i64 }\n\
        pub enum Decision { Pending, Accepted(Person) }"
        .into();
}

#[given("a Rust model containing a borrowed field")]
fn borrowed_field(world: &mut RustWorld) {
    world.driver.source = "pub struct Person { pub age: i64 }\n\
        pub struct Borrowed { pub name: &'static str }"
        .into();
}

#[given("Rust functions annotated as native and external bindings")]
fn bindings(world: &mut RustWorld) {
    world.driver.source = r#"
        #[morphir::native(hint = "arithmetic")]
        pub fn add(a: i64, b: i64) -> i64 { panic!("not executed") }
        #[morphir::external(target = "rust", name = "vendor::identity")]
        pub fn identity<T>(value: T) -> T { value }
    "#
    .into();
}

#[when("I compile the bindings to Morphir IR version 4")]
fn compile_bindings(world: &mut RustWorld) {
    world.driver.compile_with_options(CompileOptions {
        types_only: false,
        ir_version: "4".into(),
        ..Default::default()
    });
}

#[then("the IR preserves both binding kinds and their signatures")]
fn preserved_bindings(world: &mut RustWorld) {
    world.driver.assert_bindings_preserved();
}

#[when(expr = "I compile the model to Morphir IR version {string}")]
fn compile(world: &mut RustWorld, version: String) {
    world.driver.compile(&version);
}

#[when("I generate Rust from the compiled model")]
fn generate(world: &mut RustWorld) {
    world.driver.generate();
}

#[then("a Rust consumer can construct and match the generated types")]
fn usable(world: &mut RustWorld) {
    world.driver.assert_usable_rust();
}

#[then("compilation fails without partial IR")]
fn rejected(world: &mut RustWorld) {
    world.driver.assert_rejected();
}

#[tokio::main]
async fn main() {
    RustWorld::cucumber()
        .fail_on_skipped()
        .run_and_exit(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/features"))
        .await;
}
