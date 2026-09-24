//! One-shot process description with the optional-method session fallback.

use super::*;
use morphir_extension_sdk::claims::CapabilityClaimSet;
use morphir_extension_sdk::protocol::DescribeParams;
use morphir_host::probe::{check_capability_kinds, permits_fallback};
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

// `check_capability_kinds` and `permits_fallback` moved to
// `morphir_host::probe`. Their unit tests moved with them.
