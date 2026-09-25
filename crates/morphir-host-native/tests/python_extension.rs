//! Build the Python WASM guest, then run this test with `--ignored`.
//! Set MORPHIR_PYTHON_WASM to override the release artifact path.
//! The packaged lifecycle test also requires MORPHIR_PYTHON_BUNDLE.

mod support;

use morphir_extension_sdk::{
    prelude::*,
    protocol::{InitializeParams, InitializeResult, PeerInfo, methods},
};
use morphir_host::Session;
use morphir_host_native::extism::{ExtensionContainer, MorphirHostFunctions};
use support::mep::host_config;

struct PythonExtensionDriver {
    container: ExtensionContainer,
}

#[tokio::test]
#[ignore = "requires MORPHIR_PYTHON_BUNDLE from extension:artifact:python"]
async fn packaged_python_installs_and_roundtrips_offline() {
    for version in ["3", "4"] {
        packaged_roundtrip(version).await;
    }
}

async fn packaged_roundtrip(version: &str) {
    use morphir_common::home::MorphirHome;
    use morphir_distribution::{
        Channel, ExtensionId, ExtensionInstaller, LocalExtensionRepository, LocalIndex, Platform,
        Selection, activate_installed,
    };
    let bundle =
        std::env::var_os("MORPHIR_PYTHON_BUNDLE").expect("build the Python release bundle first");
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let publication = repository.publish(bundle).unwrap();
    let artifact = &publication.release().artifacts()[0];
    let claims = artifact.claims().expect("version-2 artifact claims");
    assert!(claims.capabilities.contains_key("frontend"));
    assert!(claims.capabilities.contains_key("backend"));
    assert_eq!(
        artifact.claim_check(),
        morphir_distribution::ClaimCheck::Unchecked
    );
    let id = ExtensionId::parse("morphir-python").unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let selected = LocalIndex::open(repository.root())
        .unwrap()
        .resolve(
            &id,
            Selection::Channel(Channel::Stable),
            &Platform::current(),
            &"0.4.0".parse().unwrap(),
        )
        .unwrap();
    let installed = ExtensionInstaller::new(&home)
        .install(selected, &"0.4.0".parse().unwrap())
        .unwrap();
    assert_eq!(
        serde_json::to_value(installed.claims()).unwrap(),
        serde_json::to_value(claims).unwrap()
    );
    assert_eq!(
        installed.claim_check(),
        morphir_distribution::ClaimCheck::Unchecked
    );
    // Removing this fixture's repository proves activation uses the installed copy.
    std::fs::remove_dir_all(repository.root()).unwrap();
    let guest = morphir_host_native::activate(activate_installed(&home, &id).unwrap(), root.path())
        .await
        .unwrap();
    let mut session = Session::open(
        guest.connection,
        &host_config("python-release-test", "1.0.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("negotiation failed: {error}"));
    let capabilities = session.negotiated().capabilities();
    assert_eq!(
        capabilities.frontend.as_ref().unwrap().languages[0].id,
        "python"
    );
    assert_eq!(capabilities.backend.as_ref().unwrap().targets, ["python"]);

    macro_rules! invoke {
        ($session:expr, $result:ty, $method:expr, $request:expr) => {
            match $session.call::<_, $result>($method, $request).await {
                Ok(result) => result,
                Err(morphir_host::CallError::Rejected(error)) => {
                    panic!("request rejected: {error}")
                }
                Err(error) => panic!("{} (IR {}) failed: {}", $method, version, error),
            }
        };
    }
    let source = concat!(
        include_str!("../../morphir-python-binding/tests/fixtures/models.py"),
        "\n",
        include_str!("../../morphir-python-binding/tests/fixtures/conditionals.py"),
        "\n",
        include_str!("../../morphir-python-binding/tests/fixtures/tuples.py"),
    );
    let request = |documents| CompileRequest {
        language_id: "python".into(),
        sources: SourceSet {
            root: None,
            documents,
        },
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        options: CompileOptions {
            ir_version: version.into(),
            ..Default::default()
        },
        baseline: None,
    };
    let compiled = invoke!(
        session,
        CompileResult,
        methods::COMPILE,
        request(vec![
            SourceDocument {
                uri: "functions.py".into(),
                language_id: "python".into(),
                version: 1,
                text: include_str!("../../morphir-python-binding/tests/fixtures/functions.py")
                    .into(),
            },
            SourceDocument {
                uri: "models.py".into(),
                language_id: "python".into(),
                version: 1,
                text: source.into()
            },
            SourceDocument {
                uri: "rules.py".into(),
                language_id: "python".into(),
                version: 1,
                text: include_str!("../../morphir-python-binding/tests/fixtures/modules/rules.py")
                    .into()
            },
        ])
    );
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    let ir = compiled.ir.unwrap();
    let generated = invoke!(
        session,
        GenerateResult,
        methods::GENERATE,
        GenerateRequest {
            ir: ir.clone(),
            target: "python".into(),
            options: Default::default(),
        }
    );
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert_eq!(generated.artifacts.len(), 3);
    assert!(
        generated
            .artifacts
            .iter()
            .any(|artifact| artifact.path == "functions.py" && artifact.content.contains("lambda"))
    );
    let again = invoke!(
        session,
        CompileResult,
        methods::COMPILE,
        request(
            generated
                .artifacts
                .iter()
                .map(|a| SourceDocument {
                    uri: a.path.clone(),
                    language_id: "python".into(),
                    version: 1,
                    text: a.content.clone(),
                })
                .collect()
        )
    );
    assert!(again.success, "{:?}", again.diagnostics);
    assert_eq!(again.ir, Some(ir));
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("shutdown failed: {error}"));
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
                    sources: SourceSet {
                        root: None,
                        documents: vec![SourceDocument {
                            uri: "models.py".into(),
                            language_id: "python".into(),
                            text,
                            version: 1,
                        }],
                    },
                    package: CompilePackage {
                        name: "acme/example".into(),
                        exposed_modules: None,
                    },
                    dependencies: vec![],
                    options: CompileOptions {
                        ir_version: "4.0.0".into(),
                        types_only: false,
                        ..Default::default()
                    },
                    baseline: None,
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
                    kind: Default::default(),
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
            "{}\n{}\n{}",
            include_str!("../../morphir-python-binding/tests/fixtures/models.py"),
            include_str!("../../morphir-python-binding/tests/fixtures/conditionals.py"),
            include_str!("../../morphir-python-binding/tests/fixtures/tuples.py")
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
