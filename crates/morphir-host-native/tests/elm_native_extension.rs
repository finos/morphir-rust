//! The packaged native Elm extension, installed and driven offline.
//!
//! Build the bundle first (`mise run extension:artifact:elm-native`), then run
//! this test with `--ignored`. Set MORPHIR_ELM_NATIVE_BUNDLE to the bundle the
//! artifact task staged.

mod support;

use morphir_extension_sdk::prelude::*;
use morphir_host::Session;
use support::mep::{completed, host_config};

const EXAMPLE: &str = include_str!("fixtures/morphir-elm-extension/Example.elm");
const INVALID: &str = include_str!("fixtures/morphir-elm-extension/Invalid.elm");

#[tokio::test]
#[ignore = "requires MORPHIR_ELM_NATIVE_BUNDLE from extension:artifact:elm-native"]
async fn packaged_elm_native_installs_and_compiles_offline() {
    use morphir_common::home::MorphirHome;
    use morphir_distribution::{
        Channel, ExtensionId, ExtensionInstaller, LocalExtensionRepository, LocalIndex, Platform,
        Selection, activate_installed,
    };
    let bundle = std::env::var_os("MORPHIR_ELM_NATIVE_BUNDLE")
        .expect("build the Elm native release bundle first");
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let publication = repository.publish(bundle).unwrap();
    // A version-2 bundle carries the guest's own claims. The guest reports
    // Workspace among its capability kinds at initialization, so the claims must
    // too, or negotiation below would stop on "capability kinds changed".
    let artifact = &publication.release().artifacts()[0];
    let claims = artifact.claims().expect("version-2 artifact claims");
    assert!(claims.capabilities.contains_key("frontend"));
    assert!(claims.capabilities.contains_key("backend"));
    assert!(claims.capabilities.contains_key("workspace"));
    assert!(claims.extension.types.contains(&ExtensionType::Workspace));
    assert_eq!(
        artifact.claim_check(),
        morphir_distribution::ClaimCheck::Unchecked
    );
    let id = ExtensionId::parse("morphir-elm-native").unwrap();
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
        &host_config("elm-native-release-test", "1.0.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("negotiation failed: {error}"));
    assert_eq!(session.negotiated().extension().id, "morphir-elm-native");
    let capabilities = session.negotiated().capabilities();
    let frontend = capabilities
        .frontend
        .as_ref()
        .expect("the Elm extension advertises a frontend");
    assert_eq!(frontend.languages[0].id, "elm");
    assert_eq!(frontend.languages[0].file_extensions, [".elm"]);
    assert_eq!(frontend.ir_versions, ["3", "4"]);
    assert!(frontend.incremental);
    assert_eq!(capabilities.backend.as_ref().unwrap().targets, ["elm"]);

    let request = |uri: &str, text: &str| CompileRequest {
        language_id: "elm".into(),
        sources: SourceSet {
            root: None,
            documents: vec![SourceDocument {
                uri: uri.into(),
                language_id: "elm".into(),
                version: 1,
                text: text.into(),
            }],
        },
        package: CompilePackage {
            name: "local/example".into(),
            exposed_modules: Some(vec!["Example".into()]),
        },
        dependencies: vec![],
        options: CompileOptions {
            types_only: false,
            ir_version: "3".into(),
            ..Default::default()
        },
        baseline: None,
    };

    let compiled = completed(
        "compile",
        session.compile(request("Example.elm", EXAMPLE)).await,
    );
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    assert_eq!(compiled.ir_version.as_deref(), Some("3"));
    assert!(compiled.ir.is_some(), "a successful compile returns IR");
    assert!(
        compiled.modules.iter().any(|module| module == "Example"),
        "a successful compile reports the Example module: {:?}",
        compiled.modules
    );

    let rejected = completed(
        "compile",
        session.compile(request("Invalid.elm", INVALID)).await,
    );
    assert!(!rejected.success, "malformed Elm must not compile");
    assert!(
        rejected.modules.is_empty(),
        "a module that did not compile is not reported as compiled: {:?}",
        rejected.modules
    );
    assert!(
        rejected
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error),
        "malformed Elm returns an error diagnostic: {:?}",
        rejected.diagnostics
    );
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("shutdown failed: {error}"));
}
