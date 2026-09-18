//! The packaged native Elm extension, installed and driven offline.
//!
//! Build the bundle first (`mise run extension:artifact:elm-native`), then run
//! this test with `--ignored`. Set MORPHIR_ELM_NATIVE_BUNDLE to the bundle the
//! artifact task staged.

use morphir_daemon::extensions::{InvokeOutcome, activate_transport, protocol::methods};
use morphir_extension_sdk::{
    prelude::*,
    protocol::{InitializeParams, PeerInfo},
};

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
    assert!(publication.release().frontend().is_some());
    assert!(publication.release().backend().is_some());
    let id = ExtensionId::parse("morphir-elm-native").unwrap();
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
    // Removing this fixture's repository proves activation uses the installed copy.
    std::fs::remove_dir_all(repository.root()).unwrap();
    let loaded = activate_transport(activate_installed(&home, &id).unwrap(), root.path())
        .await
        .unwrap();
    let ready = loaded
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "elm-native-release-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("negotiation failed: {}", failure.error()));
    assert_eq!(ready.negotiated().extension().id, "morphir-elm-native");
    let capabilities = ready.negotiated().capabilities();
    let frontend = capabilities
        .frontend
        .as_ref()
        .expect("the Elm extension advertises a frontend");
    assert_eq!(frontend.languages[0].id, "elm");
    assert_eq!(frontend.languages[0].file_extensions, [".elm"]);
    assert_eq!(frontend.ir_versions, ["3", "4"]);
    assert!(frontend.incremental);
    assert_eq!(capabilities.backend.as_ref().unwrap().targets, ["elm"]);

    macro_rules! invoke {
        ($ready:expr, $result:ty, $method:expr, $request:expr) => {
            match $ready.invoke::<$result>($method, $request).await {
                InvokeOutcome::Success(ready, result) => (ready, result),
                InvokeOutcome::Rejected(_, error) => panic!("request rejected: {error}"),
                InvokeOutcome::Failed(failure) => panic!("MEP failed: {}", failure.error()),
            }
        };
    }
    let request = |uri: &str, text: &str| CompileRequest {
        language_id: "elm".into(),
        documents: vec![SourceDocument {
            uri: uri.into(),
            language_id: "elm".into(),
            version: 1,
            text: text.into(),
        }],
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

    let (ready, compiled) = invoke!(
        ready,
        CompileResult,
        methods::COMPILE,
        request("Example.elm", EXAMPLE)
    );
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    assert_eq!(compiled.ir_version.as_deref(), Some("3"));
    assert!(compiled.ir.is_some(), "a successful compile returns IR");
    assert!(
        compiled.modules.iter().any(|module| module == "Example"),
        "a successful compile reports the Example module: {:?}",
        compiled.modules
    );

    let (_, rejected) = invoke!(
        ready,
        CompileResult,
        methods::COMPILE,
        request("Invalid.elm", INVALID)
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
}
