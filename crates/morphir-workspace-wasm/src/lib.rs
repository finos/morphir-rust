//! Browser-facing JSON adapter for portable Morphir workspace discovery.

use morphir_workspace::DiscoveryRequest;
use wasm_bindgen::prelude::*;

/// Runs workspace discovery from a JSON request and returns its JSON response.
pub fn discover_workspace_json(input: &str) -> Result<String, String> {
    let request: DiscoveryRequest =
        serde_json::from_str(input).map_err(|error| error.to_string())?;
    serde_json::to_string(&morphir_workspace::discover(request)).map_err(|error| error.to_string())
}

/// Returns authoritative package and discovery-protocol metadata for export tooling.
#[doc(hidden)]
pub fn workspace_metadata_json() -> String {
    serde_json::json!({
        "crateVersion": env!("CARGO_PKG_VERSION"),
        "protocolVersion": morphir_workspace::WORKSPACE_DISCOVERY_PROTOCOL,
    })
    .to_string()
}

/// Runs portable workspace discovery across the JavaScript boundary.
#[wasm_bindgen]
pub fn discover_workspace(input: &str) -> Result<String, JsError> {
    discover_workspace_json(input).map_err(|message| JsError::new(&message))
}

#[cfg(test)]
mod tests {
    use super::{discover_workspace, discover_workspace_json, workspace_metadata_json};
    use morphir_workspace::DiscoveryRequest;
    use serde::Deserialize;
    use wasm_bindgen::JsError;

    #[derive(Deserialize)]
    struct CorpusCase {
        request: DiscoveryRequest,
    }

    fn first_request(corpus: &str) -> DiscoveryRequest {
        serde_json::from_str::<Vec<CorpusCase>>(corpus)
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .request
    }

    #[test]
    fn json_boundary_matches_the_native_response() {
        let corpus = include_str!("../../../tests/fixtures/workspace-discovery/corpus.json");
        let first = first_request(corpus);
        let expected = serde_json::to_string(&morphir_workspace::discover(first.clone())).unwrap();

        assert_eq!(
            discover_workspace_json(&serde_json::to_string(&first).unwrap()).unwrap(),
            expected,
        );
    }

    #[test]
    fn invalid_json_returns_an_error_without_panicking() {
        let error = discover_workspace_json("not valid JSON").unwrap_err();

        assert!(!error.is_empty());
    }

    #[test]
    fn javascript_boundary_returns_real_errors() {
        let _: fn(&str) -> Result<String, JsError> = discover_workspace;
    }

    #[test]
    fn metadata_helper_uses_authoritative_crate_and_protocol_values() {
        let metadata: serde_json::Value = serde_json::from_str(&workspace_metadata_json()).unwrap();

        assert_eq!(metadata["crateVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            metadata["protocolVersion"],
            morphir_workspace::WORKSPACE_DISCOVERY_PROTOCOL
        );
    }
}
