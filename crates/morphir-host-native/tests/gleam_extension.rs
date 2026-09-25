//! The packaged Gleam guest installed and activated without its source repository.
//!
//! Run with MORPHIR_GLEAM_BUNDLE set to the bundle from
//! `mise run extension:artifact:gleam`, then select this test with `--ignored`.

mod support;

use morphir_extension_sdk::prelude::*;
use morphir_host::Session;
use support::mep::{completed, host_config};

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
    let artifact = &publication.release().artifacts()[0];
    let claims = artifact.claims().expect("version-2 artifact claims");
    assert!(claims.capabilities.contains_key("frontend"));
    assert!(claims.capabilities.contains_key("backend"));
    assert_eq!(
        artifact.claim_check(),
        morphir_distribution::ClaimCheck::Unchecked
    );
    assert!(claims.extension.types.contains(&ExtensionType::Workspace));
    assert!(claims.capabilities.contains_key("workspace"));
    let id = ExtensionId::parse("morphir-gleam").unwrap();
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
    // Activation must work entirely from the installed copy.
    std::fs::remove_dir_all(repository.root()).unwrap();
    let guest = morphir_host_native::activate(activate_installed(&home, &id).unwrap(), root.path())
        .await
        .unwrap();
    let mut session = Session::open(
        guest.connection,
        &host_config("gleam-release-test", "1.0.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("negotiation failed: {error}"));
    assert_eq!(session.negotiated().extension().id, "morphir-gleam");
    let capabilities = session.negotiated().capabilities();
    let frontend = capabilities.frontend.as_ref().expect("Gleam frontend");
    assert_eq!(frontend.languages[0].id, "gleam");
    assert_eq!(frontend.languages[0].file_extensions, [".gleam"]);
    assert_eq!(frontend.ir_versions, ["3", "4"]);
    assert!(frontend.incremental);
    assert_eq!(capabilities.backend.as_ref().unwrap().targets, ["gleam"]);

    let request = CompileRequest {
        language_id: "gleam".into(),
        sources: SourceSet {
            root: None,
            documents: vec![SourceDocument {
                uri: "file:///src/model.gleam".into(),
                language_id: "gleam".into(),
                version: 1,
                text: "pub type Amount = Int\npub type Color { Red Blue }\n".into(),
            }],
        },
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
    let compiled = completed("compile", session.compile(request.clone()).await);
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    assert_eq!(
        compiled.ir_version.as_deref(),
        Some(format!("{version}.0.0").as_str())
    );
    assert_eq!(compiled.modules, ["model"]);
    let generated = completed(
        "generate",
        session
            .generate(GenerateRequest {
                ir: compiled.ir.clone().unwrap(),
                target: "gleam".into(),
                options: Default::default(),
            })
            .await,
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
                frontend_state: module.frontend_state.clone(),
                name: module.name.clone(),
                uri: module.uri.clone(),
                source_digest: module.source_digest.clone().unwrap(),
                interface_digest: module.interface_digest.clone().unwrap(),
                depends_on: module.depends_on.clone(),
                ir: module.ir.clone().unwrap(),
            })
            .collect(),
    });
    let reused = completed("compile", session.compile(request).await);
    assert!(reused.success, "{:?}", reused.diagnostics);
    assert_eq!(reused.ir, compiled.ir);
    assert_eq!(reused.module_results[0].status, ModuleStatus::Unchanged);
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("shutdown failed: {error}"));
}
