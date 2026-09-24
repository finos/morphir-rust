//! One-shot process description with the optional-method session fallback.

use super::*;
use morphir_extension_sdk::claims::CapabilityClaimSet;
use morphir_extension_sdk::protocol::{DescribeParams, RpcError};
use morphir_extension_sdk::types::{ExtensionCapabilities, ExtensionType};
use serde_json::{Map, Value};

/// How a process supplied its capability claim set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptionSource {
    /// The guest answered `morphir.extension.describe` directly.
    Describe,
    /// The host reconstructed the claims from a negotiated session.
    SessionFallback,
}

/// A process claim set and the operation that produced it.
#[derive(Debug, Clone)]
pub struct ProcessDescription {
    /// The guest's claims, or the subset reported by a fallback session.
    pub claims: CapabilityClaimSet,
    /// Distinguishes direct descriptions from session reconstructions.
    pub source: DescriptionSource,
}

impl SpawnedProcessTransport {
    /// Describe a fresh process, falling back only when describe is unavailable.
    ///
    /// Consumes the transport and completes `exit` before returning. The fallback
    /// sends initialize, initialized, capabilities, shutdown, and exit in order.
    /// Launch policy and request timeouts are the same as for ordinary sessions.
    ///
    /// ```no_run
    /// # async fn example() -> morphir_daemon::Result<()> {
    /// use morphir_daemon::extensions::{ProcessLaunch, SpawnedProcessTransport};
    /// use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
    /// let transport = SpawnedProcessTransport::spawn(ProcessLaunch::new(
    ///     "example", "/extensions/example", "/workspace",
    /// )).await?;
    /// let description = transport.describe(InitializeParams {
    ///     protocol_versions: vec!["0.1".into()],
    ///     host: PeerInfo { kind: Default::default(), name: "host".into(), version: "1.0.0".into() },
    /// }).await?;
    /// assert_eq!(description.claims.extension.id, "example");
    /// # Ok(()) }
    /// ```
    pub async fn describe(mut self, params: InitializeParams) -> Result<ProcessDescription> {
        let result = self.describe_inner(params).await;
        match result {
            Ok(description) => {
                if let Err(failure) = self.terminate().await {
                    return Err(self.session.abort_with_error(failure.into_error()).await);
                }
                Ok(description)
            }
            Err(error) => Err(self.session.abort_with_error(error).await),
        }
    }

    async fn describe_inner(&mut self, params: InitializeParams) -> Result<ProcessDescription> {
        let response = self
            .description_request(
                1,
                methods::DESCRIBE,
                DescribeParams {
                    protocol_versions: params.protocol_versions.clone(),
                },
            )
            .await?;
        if response.error.as_ref().is_some_and(permits_fallback) {
            return self.describe_through_session(params).await;
        }
        let claims: CapabilityClaimSet = response.into_result(1)?;
        if claims.extension.id != self.session.expected_extension_id {
            return Err(DaemonError::Extension(
                "Description extension identity differs from launch identity".into(),
            ));
        }
        check_capability_kinds(&claims)?;
        if !claims
            .protocol_versions
            .iter()
            .any(|version| params.protocol_versions.contains(version))
        {
            return Err(DaemonError::Extension(
                "Description has no protocol version in common with the host".into(),
            ));
        }
        if claims
            .requires
            .as_ref()
            .is_some_and(|requirements| !requirements.host.is_empty())
        {
            let host = params.host.version.parse().map_err(|error| {
                DaemonError::Extension(format!("Invalid host SemVer for requires.host: {error}"))
            })?;
            claims
                .check_host(&host)
                .map_err(|error| DaemonError::Extension(error.to_string()))?;
        }
        Ok(ProcessDescription {
            claims,
            source: DescriptionSource::Describe,
        })
    }

    async fn describe_through_session(
        &mut self,
        params: InitializeParams,
    ) -> Result<ProcessDescription> {
        let initialized: InitializeResult = self
            .description_request(2, methods::INITIALIZE, &params)
            .await?
            .into_result(2)?;
        validate_negotiation(
            self.expected_extension(),
            &params.protocol_versions,
            initialized.clone(),
        )?;
        let notification = ExtensionNotification::without_params(methods::INITIALIZED);
        let stdin =
            self.session.stdin.as_mut().ok_or_else(|| {
                DaemonError::Extension("Extension process stdin is closed".into())
            })?;
        timeout(
            self.session.request_timeout,
            write_frame(stdin, &notification),
        )
        .await
        .map_err(|_| {
            DaemonError::Extension("Extension initialized notification timed out".into())
        })??;
        let capabilities: Map<String, Value> = self
            .description_request(3, methods::CAPABILITIES, serde_json::json!({}))
            .await?
            .into_result(3)?;
        let _: Value = self
            .description_request(4, methods::SHUTDOWN, serde_json::json!({}))
            .await?
            .into_result(4)?;
        Ok(ProcessDescription {
            claims: CapabilityClaimSet::from_session(
                vec![initialized.protocol_version],
                initialized.extension,
                capabilities,
            ),
            source: DescriptionSource::SessionFallback,
        })
    }

    async fn description_request(
        &mut self,
        id: u64,
        method: &str,
        params: impl Serialize,
    ) -> Result<ExtensionResponse> {
        let response = self
            .exchange(ExtensionRequest::new(method, params, id)?)
            .await
            .map_err(TransportError::into_error)?;
        response.validate_envelope(id)?;
        Ok(response)
    }
}

/// Holds a direct description to the rule a negotiated session already
/// follows: every declared kind has its capability object, every known
/// capability object has its declared kind, and each object has its wire
/// shape. A fallback claim set comes from a validated session, so only the
/// direct path needs this.
fn check_capability_kinds(claims: &CapabilityClaimSet) -> Result<()> {
    let types = &claims.extension.types;
    let unique: std::collections::HashSet<_> = types.iter().copied().collect();
    if unique.len() != types.len() {
        return Err(DaemonError::Extension(
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
                return Err(DaemonError::Extension(format!(
                    "Description declared {name} without {member} capabilities"
                )));
            }
            (false, Some(_)) => {
                return Err(DaemonError::Extension(format!(
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
        DaemonError::Extension(format!("Description capabilities are malformed: {error}"))
    })?;
    Ok(())
}

fn permits_fallback(error: &RpcError) -> bool {
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

    fn claims(types: Value, capabilities: Value) -> CapabilityClaimSet {
        serde_json::from_value(serde_json::json!({
            "claimsVersion": "0.1.0-draft.2",
            "protocolVersions": ["0.1"],
            "extension": {"id": "example", "name": "Example", "version": "1.0.0", "types": types},
            "capabilities": capabilities,
        }))
        .expect("a well-formed claim set")
    }

    fn refusal(types: Value, capabilities: Value) -> String {
        check_capability_kinds(&claims(types, capabilities))
            .expect_err("an inconsistent description is refused")
            .to_string()
    }

    #[test]
    fn declared_kinds_and_capability_objects_must_match() {
        let backend = serde_json::json!({"targets": ["x"], "irVersions": ["3"], "generate": true});
        assert!(
            check_capability_kinds(&claims(
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
            check_capability_kinds(&claims(
                serde_json::json!([]),
                serde_json::json!({"future": {"anything": 1}})
            ))
            .is_ok()
        );
    }
}
