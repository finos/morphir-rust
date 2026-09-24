//! The install probe: describe a guest before trusting its claims.

use crate::channel::{Channel, Outgoing};
use crate::envelope::{EnvelopeError, validate_envelope};
use crate::session_core::{BasicChecks, SessionChecks};
use crate::{ChannelState, HostConfig, HostError};
use morphir_extension_sdk::claims::CapabilityClaimSet;
use morphir_extension_sdk::protocol::{
    DescribeParams, ExtensionNotification, ExtensionRequest, ExtensionResponse, InitializeResult,
    RpcError, error_codes, methods,
};
use morphir_extension_sdk::{ExtensionCapabilities, ExtensionType};
use serde_json::{Map, Value};

/// How the host obtained a guest's claim set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptionSource {
    /// The guest answered `morphir.extension.describe` directly.
    Describe,
    /// The host rebuilt the claim set from a negotiated session.
    SessionFallback,
}

/// A guest's claim set and how the host obtained it.
#[derive(Debug, Clone)]
pub struct Description {
    /// The claim set.
    pub claims: CapabilityClaimSet,
    /// How the host obtained it.
    pub source: DescriptionSource,
}

/// Describe the guest behind `channel`, then release it.
///
/// A guest that refuses `morphir.extension.describe` before initialization
/// is described through a short session instead. The ids and the order of
/// messages match what released guests were tested with.
pub async fn describe<C: Channel>(
    mut channel: C,
    config: &HostConfig,
    expected_id: &str,
) -> Result<Description, HostError> {
    match describe_inner(&mut channel, config, expected_id).await {
        Ok(description) => {
            channel
                .send(Outgoing::Notification(
                    ExtensionNotification::without_params(methods::EXIT),
                ))
                .await?;
            match channel.close().await? {
                ChannelState::Stopped => Ok(description),
                ChannelState::Indeterminate => Err(HostError::Channel {
                    message: "Extension shutdown outcome is indeterminate".into(),
                    state: ChannelState::Indeterminate,
                }),
            }
        }
        Err(Failure::Channel(error)) => Err(error),
        Err(Failure::Protocol(error)) => match channel.abort().await {
            Ok(_) => Err(error),
            Err(abort) => Err(HostError::Channel {
                message: format!("{error}; transport abort also failed: {}", abort.message),
                state: abort.state,
            }),
        },
    }
}

/// Why the probe stopped. A channel failure already released the guest.
enum Failure {
    Channel(HostError),
    Protocol(HostError),
}

impl From<HostError> for Failure {
    fn from(error: HostError) -> Self {
        Failure::Protocol(error)
    }
}

async fn exchange<C: Channel>(
    channel: &mut C,
    id: u64,
    method: &str,
    params: impl serde::Serialize,
) -> Result<ExtensionResponse, Failure> {
    let request = ExtensionRequest::new(method, params, id).map_err(HostError::from)?;
    channel
        .send(Outgoing::Request(request))
        .await
        .map_err(|error| Failure::Channel(error.into()))?;
    channel
        .receive()
        .await
        .map_err(|error| Failure::Channel(error.into()))
}

fn result<T: serde::de::DeserializeOwned>(
    response: ExtensionResponse,
    id: u64,
) -> Result<T, Failure> {
    match validate_envelope(response, id) {
        Ok(value) => Ok(serde_json::from_value(value).map_err(HostError::from)?),
        Err(EnvelopeError::Rpc(error) | EnvelopeError::Invalid(error)) => Err(error.into()),
    }
}

async fn describe_inner<C: Channel>(
    channel: &mut C,
    config: &HostConfig,
    expected_id: &str,
) -> Result<Description, Failure> {
    let offered = config.protocol_versions().to_vec();
    let response = exchange(
        channel,
        1,
        methods::DESCRIBE,
        DescribeParams {
            protocol_versions: offered.clone(),
        },
    )
    .await?;
    if response.id == 1 && response.error.as_ref().is_some_and(permits_fallback) {
        return describe_through_session(channel, config, expected_id).await;
    }
    let claims: CapabilityClaimSet = result(response, 1)?;
    if claims.extension.id != expected_id {
        return Err(HostError::Invalid(
            "Description extension identity differs from launch identity".into(),
        )
        .into());
    }
    check_capability_kinds(&claims)?;
    if !claims
        .protocol_versions
        .iter()
        .any(|version| offered.contains(version))
    {
        return Err(HostError::Invalid(
            "Description has no protocol version in common with the host".into(),
        )
        .into());
    }
    if claims
        .requires
        .as_ref()
        .is_some_and(|requirements| !requirements.host.is_empty())
    {
        let host = config.peer().version.parse().map_err(|error| {
            HostError::Invalid(format!("Invalid host SemVer for requires.host: {error}"))
        })?;
        claims
            .check_host(&host)
            .map_err(|error| HostError::Invalid(error.to_string()))?;
    }
    Ok(Description {
        claims,
        source: DescriptionSource::Describe,
    })
}

async fn describe_through_session<C: Channel>(
    channel: &mut C,
    config: &HostConfig,
    expected_id: &str,
) -> Result<Description, Failure> {
    let params = config.initialize_params();
    let initialized: InitializeResult =
        result(exchange(channel, 2, methods::INITIALIZE, &params).await?, 2)?;
    BasicChecks::new(expected_id).negotiate(&params.protocol_versions, initialized.clone())?;
    channel
        .send(Outgoing::Notification(
            ExtensionNotification::without_params(methods::INITIALIZED),
        ))
        .await
        .map_err(|error| Failure::Channel(error.into()))?;
    let capabilities: Map<String, Value> = result(
        exchange(channel, 3, methods::CAPABILITIES, serde_json::json!({})).await?,
        3,
    )?;
    let _: Value = result(
        exchange(channel, 4, methods::SHUTDOWN, serde_json::json!({})).await?,
        4,
    )?;
    Ok(Description {
        claims: CapabilityClaimSet::from_session(
            vec![initialized.protocol_version],
            initialized.extension,
            capabilities,
        ),
        source: DescriptionSource::SessionFallback,
    })
}

/// Holds a direct description to the rule a negotiated session already
/// follows: every declared kind has its capability object, every known
/// capability object has its declared kind, and each object has its wire
/// shape. A fallback claim set comes from a validated session, so only the
/// direct path needs this.
#[doc(hidden)]
pub fn check_capability_kinds(claims: &CapabilityClaimSet) -> Result<(), HostError> {
    let types = &claims.extension.types;
    let unique: std::collections::HashSet<_> = types.iter().copied().collect();
    if unique.len() != types.len() {
        return Err(HostError::Invalid(
            "Description repeated a capability kind".into(),
        ));
    }
    let capabilities = &claims.capabilities;
    for (kind, member, name) in [
        (ExtensionType::Frontend, "frontend", "Frontend"),
        (ExtensionType::Backend, "backend", "Backend"),
        (ExtensionType::Workspace, "workspace", "Workspace"),
    ] {
        match (unique.contains(&kind), capabilities.get(member)) {
            (true, None) => {
                return Err(HostError::Invalid(format!(
                    "Description declared {name} without {member} capabilities"
                )));
            }
            (false, Some(_)) => {
                return Err(HostError::Invalid(format!(
                    "Description advertised {member} capabilities without declaring {name}"
                )));
            }
            _ => {}
        }
    }
    let known: Map<String, Value> = ["frontend", "backend", "workspace"]
        .into_iter()
        .filter_map(|member| {
            capabilities
                .get(member)
                .map(|value| (member.to_owned(), value.clone()))
        })
        .collect();
    serde_json::from_value::<ExtensionCapabilities>(Value::Object(known)).map_err(|error| {
        HostError::Invalid(format!("Description capabilities are malformed: {error}"))
    })?;
    Ok(())
}

#[doc(hidden)]
pub fn permits_fallback(error: &RpcError) -> bool {
    if error.code == error_codes::METHOD_NOT_FOUND || error.code == error_codes::NOT_INITIALIZED {
        return true;
    }
    // Guests released before MEP assigned `-32014` refuse with a message.
    // Recognize explicit lifecycle refusals from them, without treating
    // internal errors as optional methods.
    if error.code != error_codes::INVALID_REQUEST && !(-32099..=-32000).contains(&error.code) {
        return false;
    }
    let message = error.message.to_ascii_lowercase();
    [
        "not initialized",
        "not initialised",
        "before initialize",
        "before initialization",
        "before morphir.initialize",
        "initialize first",
        "initialization required",
        "must be initialized",
        "initialize must be called",
    ]
    .iter()
    .any(|phrase| message.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MemoryChannel, frontend_initialize_result};
    use morphir_extension_sdk::claims::CapabilityClaimSet;
    use morphir_extension_sdk::protocol::{
        ExtensionResponse, PeerInfo, PeerKind, RpcError, error_codes, methods,
    };
    use serde_json::json;

    fn config() -> HostConfig {
        HostConfig::new(PeerInfo {
            kind: PeerKind::Unspecified,
            name: "test".into(),
            version: "1.0.0".into(),
        })
    }

    fn claims(id: &str) -> CapabilityClaimSet {
        let init = frontend_initialize_result(id);
        CapabilityClaimSet::from_metadata(
            vec![init.protocol_version],
            init.extension,
            &init.capabilities,
        )
        .unwrap()
    }

    fn ok(id: u64, value: impl serde::Serialize) -> ExtensionResponse {
        ExtensionResponse::success(id, value).unwrap()
    }

    fn rpc(id: u64, code: i32, message: &str) -> ExtensionResponse {
        ExtensionResponse::error(
            id,
            RpcError {
                code,
                message: message.into(),
                data: None,
            },
        )
    }

    #[tokio::test]
    async fn a_direct_description_is_returned_and_the_guest_exits() {
        let channel = MemoryChannel::new().respond(ok(1, claims("guest")));
        let log = channel.log();
        let description = describe(channel, &config(), "guest").await.unwrap();
        assert_eq!(description.source, DescriptionSource::Describe);
        assert_eq!(description.claims.extension.id, "guest");
        assert_eq!(log.methods(), [methods::DESCRIBE, methods::EXIT]);
        assert_eq!(log.closes(), 1);
    }

    #[tokio::test]
    async fn fallback_uses_the_released_ids_and_order() {
        let channel = MemoryChannel::new()
            .respond(rpc(1, error_codes::METHOD_NOT_FOUND, "no"))
            .respond(ok(2, frontend_initialize_result("guest")))
            .respond(ok(3, json!({"frontend": {"languages": [], "irVersions": [], "compile": true, "incremental": false, "fragments": false}})))
            .respond(ok(4, json!({})));
        let log = channel.log();
        let description = describe(channel, &config(), "guest").await.unwrap();
        assert_eq!(description.source, DescriptionSource::SessionFallback);
        assert_eq!(
            log.methods(),
            [
                methods::DESCRIBE,
                methods::INITIALIZE,
                methods::INITIALIZED,
                methods::CAPABILITIES,
                methods::SHUTDOWN,
                methods::EXIT
            ]
        );
        assert_eq!(log.ids(), [1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn a_legacy_refusal_message_falls_back() {
        let channel = MemoryChannel::new()
            .respond(rpc(
                1,
                error_codes::INVALID_REQUEST,
                "Extension is not initialized",
            ))
            .respond(ok(2, frontend_initialize_result("guest")))
            .respond(ok(3, json!({})))
            .respond(ok(4, json!({})));
        let description = describe(channel, &config(), "guest").await.unwrap();
        assert_eq!(description.source, DescriptionSource::SessionFallback);
    }

    #[tokio::test]
    async fn an_internal_error_does_not_fall_back_and_aborts() {
        let channel = MemoryChannel::new().respond(rpc(1, error_codes::INTERNAL_ERROR, "boom"));
        let log = channel.log();
        let error = describe(channel, &config(), "guest").await.unwrap_err();
        assert_eq!(error.to_string(), "RPC error -32603: boom");
        assert_eq!(log.aborts(), 1);
        assert_eq!(log.methods(), [methods::DESCRIBE]);
    }

    #[tokio::test]
    async fn a_different_identity_is_refused() {
        let channel = MemoryChannel::new().respond(ok(1, claims("impostor")));
        let error = describe(channel, &config(), "guest").await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Description extension identity differs from launch identity"
        );
    }

    fn kind_claims(types: Value, capabilities: Value) -> CapabilityClaimSet {
        serde_json::from_value(serde_json::json!({
            "claimsVersion": "0.1.0-draft.2",
            "protocolVersions": ["0.1"],
            "extension": {"id": "example", "name": "Example", "version": "1.0.0", "types": types},
            "capabilities": capabilities,
        }))
        .expect("a well-formed claim set")
    }

    fn refusal(types: Value, capabilities: Value) -> String {
        check_capability_kinds(&kind_claims(types, capabilities))
            .expect_err("an inconsistent description is refused")
            .to_string()
    }

    #[test]
    fn declared_kinds_and_capability_objects_must_match() {
        let backend = serde_json::json!({"targets": ["x"], "irVersions": ["3"], "generate": true});
        assert!(
            check_capability_kinds(&kind_claims(
                serde_json::json!(["backend"]),
                serde_json::json!({"backend": backend.clone()})
            ))
            .is_ok()
        );
        assert!(
            refusal(serde_json::json!(["backend"]), serde_json::json!({}))
                .contains("declared Backend without backend capabilities")
        );
        assert!(
            refusal(
                serde_json::json!([]),
                serde_json::json!({"backend": backend})
            )
            .contains("advertised backend capabilities without declaring Backend")
        );
        assert!(
            refusal(serde_json::json!(["workspace"]), serde_json::json!({}))
                .contains("declared Workspace without workspace capabilities")
        );
    }

    #[test]
    fn repeated_kinds_and_malformed_objects_are_refused() {
        let workspace =
            serde_json::json!({"discover": true, "protocolVersions": ["0.1.0-draft.1"]});
        assert!(
            refusal(
                serde_json::json!(["workspace", "workspace"]),
                serde_json::json!({"workspace": workspace})
            )
            .contains("repeated a capability kind")
        );
        assert!(
            refusal(
                serde_json::json!(["frontend"]),
                serde_json::json!({"frontend": {"compile": "yes"}})
            )
            .contains("malformed")
        );
    }

    #[test]
    fn unknown_capability_members_are_not_kinds() {
        assert!(
            check_capability_kinds(&kind_claims(
                serde_json::json!([]),
                serde_json::json!({"future": {"anything": 1}})
            ))
            .is_ok()
        );
    }
}
