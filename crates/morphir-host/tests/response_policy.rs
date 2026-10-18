use morphir_extension_sdk::protocol::methods;
use morphir_host::GuestResponsePolicy;

#[test]
fn artifact_policy_only_expands_the_generate_response() {
    let legacy = GuestResponsePolicy::Legacy;
    let artifacts = GuestResponsePolicy::GeneratedArtifacts;
    for method in [
        methods::INITIALIZE,
        methods::COMPILE,
        methods::SHUTDOWN,
        "morphir.generateModel",
        methods::GENERATE_MODEL,
        "morphir.generate",
        "other.generate",
        "",
    ] {
        assert_eq!(artifacts.response_limit(method).bytes(), 64 * 1024 * 1024);
    }
    assert_eq!(
        legacy.response_limit(methods::GENERATE).bytes(),
        64 * 1024 * 1024
    );
    assert_eq!(
        artifacts.response_limit(methods::GENERATE).bytes(),
        385 * 1024 * 1024
    );
    assert_eq!(GuestResponsePolicy::default(), legacy);
}

#[test]
fn response_bounds_accept_exactly_and_reject_one_over() {
    for policy in [
        GuestResponsePolicy::Legacy,
        GuestResponsePolicy::GeneratedArtifacts,
    ] {
        let limit = policy.response_limit(methods::GENERATE);
        assert!(limit.check_len(limit.bytes() as usize).is_ok());
        assert!(limit.check_len(limit.bytes() as usize + 1).is_err());
    }
}

#[tokio::test]
async fn sources_default_to_legacy_and_refuse_an_unsupported_policy() {
    use morphir_extension_sdk::{BackendCapability, ExtensionCapabilities, ExtensionInfo};
    use morphir_host::testing::FakeSource;
    use morphir_host::{GuestSource, InvocationPolicy, Registry};
    use std::{path::Path, sync::Arc};
    let source = Arc::new(FakeSource::builtin(
        ExtensionInfo {
            id: "legacy-source".into(),
            ..Default::default()
        },
        ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["test".into()],
                ir_versions: vec!["4".into()],
                generate: true,
            }),
            ..Default::default()
        },
    ));
    assert!(
        source
            .connect_with_response_policy(Path::new("."), GuestResponsePolicy::Legacy)
            .await
            .is_ok()
    );
    assert!(
        source
            .connect_with_response_policy(Path::new("."), GuestResponsePolicy::GeneratedArtifacts)
            .await
            .is_err()
    );
    let mut registry = Registry::new();
    registry.register(source).unwrap();
    let resolved = registry
        .resolve_backend("test", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert!(
        resolved
            .connect_with_response_policy(Path::new("."), GuestResponsePolicy::Legacy)
            .await
            .is_ok()
    );
    let error = resolved
        .connect_with_response_policy(Path::new("."), GuestResponsePolicy::GeneratedArtifacts)
        .await
        .err()
        .expect("unsupported opt-in must not silently become legacy");
    assert!(error.to_string().contains("response policy"));
}
