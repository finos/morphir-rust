//! Build the Python WASM guest, then run this test with `--ignored`.
//! Set MORPHIR_PYTHON_WASM to override the release artifact path.

mod support;

use morphir_daemon::{
    ExtensionContainer,
    extensions::{host_functions::MorphirHostFunctions, protocol::methods},
};
use morphir_extension_sdk::{
    prelude::*,
    protocol::{InitializeParams, InitializeResult, PeerInfo},
};

struct PythonExtensionDriver {
    container: ExtensionContainer,
}

impl PythonExtensionDriver {
    fn load() -> Self {
        let path = std::env::var_os("MORPHIR_PYTHON_WASM")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                support::installed_wasm::wasm_guest_path("morphir_python_binding.wasm")
            });
        Self {
            container: ExtensionContainer::new(
                "morphir-python",
                &path,
                MorphirHostFunctions::default(),
            )
            .unwrap(),
        }
    }

    async fn compile(&self, text: String) -> CompileResult {
        self.container
            .call(
                methods::COMPILE,
                CompileRequest {
                    language_id: "python".into(),
                    documents: vec![SourceDocument {
                        uri: "models.py".into(),
                        language_id: "python".into(),
                        text,
                        version: 1,
                    }],
                    package: CompilePackage {
                        name: "acme/example".into(),
                        exposed_modules: vec![],
                    },
                    dependencies: vec![],
                    options: CompileOptions {
                        ir_version: "4.0.0".into(),
                        types_only: false,
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires the independently built morphir-python-binding WASM guest"]
async fn python_adt_and_conditional_roundtrip_through_the_real_wasm_extension() {
    let driver = PythonExtensionDriver::load();
    let initialized: InitializeResult = driver
        .container
        .call(
            methods::INITIALIZE,
            InitializeParams {
                protocol_versions: vec!["0.1".into()],
                host: PeerInfo {
                    name: "python-test".into(),
                    version: "1.0.0".into(),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(initialized.extension.id, "morphir-python");
    assert!(initialized.capabilities.frontend.unwrap().compile);
    assert!(initialized.capabilities.backend.unwrap().generate);
    let compiled = driver
        .compile(format!(
            "{}\n{}",
            include_str!("../../morphir-python-binding/tests/fixtures/models.py"),
            include_str!("../../morphir-python-binding/tests/fixtures/conditionals.py")
        ))
        .await;
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    let ir = compiled.ir.unwrap();
    assert!(ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"]["choose"]["Public"]["ExpressionBody"]["body"].get("IfThenElse").is_some());
    let generated: GenerateResult = driver
        .container
        .call(
            methods::GENERATE,
            GenerateRequest {
                ir: ir.clone(),
                target: "python".into(),
                options: Default::default(),
            },
        )
        .await
        .unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert_eq!(generated.artifacts[0].path, "models.py");
    let recompiled = driver.compile(generated.artifacts[0].content.clone()).await;
    assert!(recompiled.success, "{:?}", recompiled.diagnostics);
    assert_eq!(recompiled.ir.unwrap(), ir);
}
