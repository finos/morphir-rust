//! The packaged Gleam guest installed and activated without its source repository.
//!
//! Run with MORPHIR_GLEAM_BUNDLE set to the bundle from
//! `mise run extension:artifact:gleam`, then select this test with `--ignored`.
use morphir_daemon::extensions::{InvokeOutcome, activate_transport, protocol::methods};
use morphir_extension_sdk::{
    prelude::*,
    protocol::{InitializeParams, PeerInfo},
};

#[tokio::test]
#[ignore = "requires MORPHIR_GLEAM_BUNDLE from extension:artifact:gleam"]
async fn packaged_gleam_installs_compiles_generates_and_reuses_offline() {
    for version in ["3", "4"] {
        installed_compilation(version).await;
    }
}

async fn installed_compilation(version: &str) {
    use morphir_common::home::MorphirHome;
    use morphir_distribution::{
        Channel, ExtensionId, ExtensionInstaller, LocalExtensionRepository, LocalIndex, Platform,
        Selection, activate_installed,
    };
    let bundle =
        std::env::var_os("MORPHIR_GLEAM_BUNDLE").expect("build the Gleam release bundle first");
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let publication = repository.publish(bundle).unwrap();
    assert!(publication.release().frontend().is_some());
    assert!(publication.release().backend().is_some());
    let id = ExtensionId::parse("morphir-gleam-binding").unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let selected = LocalIndex::open(repository.root())
        .unwrap()
        .resolve(
            &id,
            Selection::Channel(Channel::Stable),
            &Platform::current(),
        )
        .unwrap();
    ExtensionInstaller::new(&home).install(selected).unwrap();
    // Activation must work entirely from the installed copy.
    std::fs::remove_dir_all(repository.root()).unwrap();
    let loaded = activate_transport(activate_installed(&home, &id).unwrap(), root.path())
        .await
        .unwrap();
    let ready = loaded
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "gleam-release-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("negotiation failed: {}", failure.error()));
    assert_eq!(ready.negotiated().extension().id, "morphir-gleam-binding");
    let capabilities = ready.negotiated().capabilities();
    let frontend = capabilities.frontend.as_ref().expect("Gleam frontend");
    assert_eq!(frontend.languages[0].id, "gleam");
    assert_eq!(frontend.languages[0].file_extensions, [".gleam"]);
    assert_eq!(frontend.ir_versions, ["3", "4"]);
    assert!(frontend.incremental);
    assert_eq!(capabilities.backend.as_ref().unwrap().targets, ["gleam"]);

    macro_rules! invoke {
        ($ready:expr, $result:ty, $method:expr, $request:expr) => {
            match $ready.invoke::<$result>($method, $request).await {
                InvokeOutcome::Success(ready, result) => (ready, result),
                InvokeOutcome::Rejected(_, error) => panic!("request rejected: {error}"),
                InvokeOutcome::Failed(failure) => panic!("MEP failed: {}", failure.error()),
            }
        };
    }
    let request = CompileRequest {
        language_id: "gleam".into(),
        documents: vec![SourceDocument {
            uri: "file:///src/model.gleam".into(),
            language_id: "gleam".into(),
            version: 1,
            text: "pub type Amount = Int\npub type Color { Red Blue }\n".into(),
        }],
        package: CompilePackage {
            name: "sample".into(),
            exposed_modules: None,
        },
        options: CompileOptions {
            ir_version: version.into(),
            types_only: true,
            extra: [("emitParseStage".into(), false.into())].into(),
        },
        ..Default::default()
    };
    let (ready, compiled) = invoke!(ready, CompileResult, methods::COMPILE, request.clone());
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    assert_eq!(
        compiled.ir_version.as_deref(),
        Some(format!("{version}.0.0").as_str())
    );
    assert_eq!(compiled.modules, ["model"]);
    let (ready, generated) = invoke!(
        ready,
        GenerateResult,
        methods::GENERATE,
        GenerateRequest {
            ir: compiled.ir.clone().unwrap(),
            target: "gleam".into(),
            options: Default::default(),
        }
    );
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert_eq!(generated.artifacts.len(), 1);
    assert!(generated.artifacts[0].content.contains("pub type Amount"));
    let mut request = request;
    request.baseline = Some(CompileBaseline {
        context_digest: compiled.context_digest.clone(),
        modules: compiled
            .module_results
            .iter()
            .map(|module| BaselineModule {
                name: module.name.clone(),
                uri: module.uri.clone(),
                source_digest: module.source_digest.clone().unwrap(),
                interface_digest: module.interface_digest.clone().unwrap(),
                depends_on: module.depends_on.clone(),
                ir: module.ir.clone().unwrap(),
            })
            .collect(),
    });
    let (_, reused) = invoke!(ready, CompileResult, methods::COMPILE, request);
    assert!(reused.success, "{:?}", reused.diagnostics);
    assert_eq!(reused.ir, compiled.ir);
    assert_eq!(reused.module_results[0].status, ModuleStatus::Unchanged);
}
