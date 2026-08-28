//! Conformance test for the independently built Morphir Elm MEP extension.
//!
//! Build the extension in `finos/morphir-elm`, then provide its path before
//! running this ignored test:
//!
//! `MORPHIR_ELM_EXTENSION_BIN=/path/to/morphir-elm-extension cargo test -p morphir-daemon --test morphir_elm_extension -- --ignored`

mod support;

use morphir_daemon::extensions::ProcessLaunch;
use morphir_extension_sdk::{CompileOptions, CompilePackage, CompileRequest, SourceDocument};
use std::path::PathBuf;

fn extension_path() -> PathBuf {
    std::env::var_os("MORPHIR_ELM_EXTENSION_BIN")
        .map(PathBuf::from)
        .expect("MORPHIR_ELM_EXTENSION_BIN should point at morphir-elm-extension")
}

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/morphir-elm-extension")
}

fn an_elm_request(uri: &str, text: &str) -> CompileRequest {
    CompileRequest {
        language_id: "elm".into(),
        documents: vec![SourceDocument {
            uri: uri.into(),
            language_id: "elm".into(),
            version: 1,
            text: text.into(),
        }],
        package: CompilePackage {
            name: "local/example".into(),
            exposed_modules: vec!["Example".into()],
        },
        dependencies: Vec::new(),
        options: CompileOptions {
            types_only: false,
            ir_version: "3".into(),
            extra: Default::default(),
        },
    }
}

#[tokio::test]
#[ignore = "requires the independently built morphir-elm-extension executable"]
async fn conforms_to_the_mep_frontend_process_contract() {
    let fixtures = fixture_directory();
    let launch = ProcessLaunch::new("morphir-elm", extension_path(), &fixtures);
    let valid_request = an_elm_request(
        "file:///conformance/Example.elm",
        include_str!("fixtures/morphir-elm-extension/Example.elm"),
    );
    let malformed_request = an_elm_request(
        "file:///conformance/Invalid.elm",
        include_str!("fixtures/morphir-elm-extension/Invalid.elm"),
    );

    support::mep::assert_frontend_typestate_conformance(launch, valid_request, malformed_request)
        .await;
}
