//! Exercise the installed Rust release bundle through MEP and executable consumers.
//! Build with `extension:artifact:rust`, then set MORPHIR_RUST_BUNDLE and run
//! `cargo test -p morphir-daemon --test rust_extension -- --ignored`.

#[path = "../../morphir-rust-binding/tests/support/functions.rs"]
mod functions;
#[path = "../../morphir-rust-binding/tests/support/pattern.rs"]
mod pattern;

use morphir_common::home::MorphirHome;
use morphir_daemon::extensions::{InvokeOutcome, activate_transport, protocol::methods};
use morphir_distribution::{
    Channel, ExtensionId, ExtensionInstaller, LocalExtensionRepository, LocalIndex, Platform,
    Selection, activate_installed,
};
use morphir_extension_sdk::{
    prelude::*,
    protocol::{InitializeParams, PeerInfo},
};

macro_rules! invoke {
    ($ready:expr, $result:ty, $method:expr, $request:expr) => {
        match $ready.invoke::<$result>($method, $request).await {
            InvokeOutcome::Success(ready, result) => (ready, result),
            InvokeOutcome::Rejected(_, error) => panic!("request rejected: {error}"),
            InvokeOutcome::Failed(failure) => panic!("MEP failed: {}", failure.error()),
        }
    };
}

#[tokio::test]
#[ignore = "requires MORPHIR_RUST_BUNDLE from extension:artifact:rust"]
async fn packaged_rust_installs_and_executes_offline_in_both_ir_versions() {
    let bundle = std::env::var_os("MORPHIR_RUST_BUNDLE")
        .expect("set MORPHIR_RUST_BUNDLE to the Rust release bundle first");
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let publication = repository.publish(bundle).unwrap();
    assert_eq!(publication.release().name(), "Morphir Rust");
    let published_version = publication.release().version().to_string();
    assert!(publication.release().frontend().is_some());
    assert!(publication.release().backend().is_some());

    let id = ExtensionId::parse("morphir-rust").unwrap();
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
    ExtensionInstaller::new(&home)
        .install(selected, &"0.4.0".parse().unwrap())
        .unwrap();
    // Activation must use the installed artifact after the repository is gone.
    std::fs::remove_dir_all(repository.root()).unwrap();
    let loaded = activate_transport(activate_installed(&home, &id).unwrap(), root.path())
        .await
        .unwrap();
    let mut ready = loaded
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "rust-release-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("negotiation failed: {}", failure.error()));

    let negotiated = ready.negotiated();
    assert_eq!(negotiated.protocol_version(), "0.1");
    assert_eq!(negotiated.extension().id, "morphir-rust");
    assert_eq!(negotiated.extension().name, "Morphir Rust");
    assert_eq!(negotiated.extension().version, published_version);
    assert!(
        negotiated
            .extension()
            .types
            .contains(&ExtensionType::Frontend)
    );
    assert!(
        negotiated
            .extension()
            .types
            .contains(&ExtensionType::Backend)
    );
    let capabilities = negotiated.capabilities();
    let frontend = capabilities.frontend.as_ref().unwrap();
    assert!(frontend.compile);
    assert_eq!(frontend.languages.len(), 1);
    assert_eq!(frontend.languages[0].id, "rust");
    assert_eq!(frontend.languages[0].file_extensions, [".rs"]);
    assert_eq!(frontend.ir_versions, ["3", "4"]);
    let backend = capabilities.backend.as_ref().unwrap();
    assert!(backend.generate);
    assert_eq!(backend.targets, ["rust"]);
    assert_eq!(backend.ir_versions, ["3", "4"]);

    let source = format!("{}\n{}", functions::SOURCE, pattern::SOURCE);
    for version in ["3", "4"] {
        let (session, compiled) = invoke!(
            ready,
            CompileResult,
            methods::COMPILE,
            compile_request(&source, version)
        );
        assert!(compiled.success, "v{version}: {:?}", compiled.diagnostics);
        assert!(
            compiled.diagnostics.is_empty(),
            "{:?}",
            compiled.diagnostics
        );
        let ir = compiled.ir.expect("successful compilation must return IR");
        assert_eq!(ir["formatVersion"], version.parse::<u64>().unwrap());
        let (session, generated) = invoke!(
            session,
            GenerateResult,
            methods::GENERATE,
            GenerateRequest {
                ir,
                target: "rust".into(),
                options: Default::default(),
            }
        );
        assert!(generated.success, "v{version}: {:?}", generated.diagnostics);
        assert!(
            generated.diagnostics.is_empty(),
            "{:?}",
            generated.diagnostics
        );
        assert_eq!(generated.artifacts.len(), 1);
        assert_eq!(generated.artifacts[0].path, "lib.rs");
        // Each consumer compiles generated Rust with rustc, then executes fixed
        // assertions for types, matches, conditionals, calls and Copy captures.
        functions::assert_executable(&generated.artifacts[0].content);
        pattern::assert_executable(&generated.artifacts[0].content);

        let (session, unsupported) = invoke!(
            session,
            CompileResult,
            methods::COMPILE,
            compile_request(
                "pub fn owned(value: String) -> String { let get = || value; get() }",
                version
            )
        );
        assert!(!unsupported.success, "v{version} accepted an owned capture");
        assert!(unsupported.ir.is_none());
        assert!(unsupported.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error && !diagnostic.message.is_empty()
        }));
        ready = session;
    }
    ready
        .shutdown()
        .await
        .unwrap_or_else(|failure| panic!("shutdown failed: {}", failure.error()));
}

fn compile_request(source: &str, version: &str) -> CompileRequest {
    CompileRequest {
        language_id: "rust".into(),
        sources: SourceSet {
            root: None,
            documents: vec![SourceDocument {
                uri: "models.rs".into(),
                language_id: "rust".into(),
                version: 1,
                text: source.into(),
            }],
        },
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: Some(vec!["Models".into()]),
        },
        options: CompileOptions {
            types_only: false,
            ir_version: version.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
