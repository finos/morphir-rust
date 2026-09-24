//! Validation of initialization data.
//!
//! Method results are checked by `morphir_host_native::validate_result`.

use super::controller::NegotiatedSession;
use super::transport::ExpectedExtension;
use crate::extensions::protocol::InitializeResult;
use crate::{DaemonError, Result};

pub(in crate::extensions) fn validate_negotiation(
    expected: ExpectedExtension,
    offered_versions: &[String],
    result: InitializeResult,
) -> Result<NegotiatedSession> {
    morphir_host::validate_negotiation(expected, offered_versions, result).map_err(Into::into)
}

/// The daemon's negotiation rules, run by the portable session core.
#[derive(Debug)]
pub(super) struct DaemonChecks {
    expected: ExpectedExtension,
}

impl DaemonChecks {
    pub(super) fn new(expected: ExpectedExtension) -> Self {
        Self { expected }
    }
}

impl morphir_host::SessionChecks for DaemonChecks {
    type Error = DaemonError;

    fn negotiate(
        &mut self,
        offered: &[String],
        result: InitializeResult,
    ) -> Result<NegotiatedSession> {
        validate_negotiation(self.expected.clone(), offered, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::PersistedExtensionCapabilities;
    use morphir_distribution::InstalledExtension;
    use serde_json::json;

    fn an_installed_frontend(flags: bool) -> InstalledExtension {
        serde_json::from_value(json!({
            "extensionId": "sample", "name": "Sample", "version": "1.0.0",
            "runtime": "wasm", "platform": null, "args": [],
            "digest": "a".repeat(64), "storePath": "extensions/sample.wasm",
            "executable": false,
            "index": {"kind": "local-directory", "identity": "/tmp/index", "revision": "b".repeat(64)},
            "claims": {
                "claimsVersion": "0.1.0-draft.2", "protocolVersions": ["0.1"],
                "extension": {"id": "sample", "name": "Sample", "version": "1.0.0", "types": ["frontend"]},
                "capabilities": {"frontend": {
                    "languages": [{"id": "elm", "fileExtensions": [".elm"]}],
                    "irVersions": ["3"], "compile": true,
                    "multiDocument": flags, "fragments": flags, "incremental": flags
                }}
            }
        })).unwrap()
    }

    fn expected(installed: &InstalledExtension) -> ExpectedExtension {
        let capabilities = installed.extension_capabilities();
        ExpectedExtension::discovered_with_persisted_capabilities(
            installed.extension_info(),
            PersistedExtensionCapabilities::from_claims(
                capabilities.frontend,
                capabilities.backend,
            ),
        )
    }

    fn reported(installed: &InstalledExtension) -> InitializeResult {
        InitializeResult {
            protocol_version: "0.1".into(),
            extension: installed.extension_info(),
            capabilities: serde_json::from_value(
                serde_json::to_value(&installed.claims().capabilities).unwrap(),
            )
            .unwrap(),
        }
    }

    #[test]
    fn installed_frontend_claims_accept_a_matching_session() {
        for flags in [false, true] {
            let installed = an_installed_frontend(flags);
            let negotiated =
                validate_negotiation(expected(&installed), &["0.1".into()], reported(&installed));
            assert!(negotiated.is_ok(), "{negotiated:?}");
        }
    }

    #[test]
    fn installed_frontend_claims_reject_session_flag_drift() {
        for flags in [false, true] {
            for member in ["multiDocument", "fragments", "incremental"] {
                let installed = an_installed_frontend(flags);
                let mut result = reported(&installed);
                let frontend = result.capabilities.frontend.as_mut().unwrap();
                match member {
                    "multiDocument" => frontend.multi_document = !flags,
                    "fragments" => frontend.fragments = !flags,
                    "incremental" => frontend.incremental = !flags,
                    _ => unreachable!(),
                }
                let error = validate_negotiation(expected(&installed), &["0.1".into()], result)
                    .expect_err("persisted frontend flags must agree with the session");
                assert!(error.to_string().contains(member), "{error}");
            }
        }
    }

    #[test]
    fn legacy_frontend_leaves_multi_document_to_the_session() {
        let installed = an_installed_frontend(false);
        let capabilities = installed.extension_capabilities();
        let expected = ExpectedExtension::discovered_with_persisted_capabilities(
            installed.extension_info(),
            PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend),
        );
        let mut result = reported(&installed);
        result
            .capabilities
            .frontend
            .as_mut()
            .unwrap()
            .multi_document = true;
        assert!(validate_negotiation(expected, &["0.1".into()], result).is_ok());
    }
}
