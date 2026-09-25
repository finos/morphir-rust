//! Starting an installed guest from its verified artifact.

use crate::extism::{ExtensionContainer, ExtismChannel, MorphirHostFunctions};
use crate::process::{ProcessChannel, ProcessLaunch};
use crate::results::CheckedConnection;
use morphir_distribution::VerifiedExtensionArtifact;
use morphir_host::{
    Channel, ExpectedChecks, ExpectedExtension, HostError, JsonRpcConnection,
    PersistedExtensionCapabilities,
};
use std::path::Path;

/// A started guest, ready for the MEP handshake.
pub struct ActivatedGuest {
    /// The guest's connection. Every call result is checked.
    pub connection: CheckedConnection<JsonRpcConnection<Box<dyn Channel>, ExpectedChecks>>,
    /// The extension id the installed metadata records.
    pub id: String,
    /// What the handshake holds the guest to: the installed identity and
    /// the capabilities the installed metadata locks.
    pub expectation: ExpectedExtension,
}

/// Start the guest in `artifact` without running the handshake.
///
/// A process artifact is staged and spawned with its recorded arguments. A
/// WebAssembly artifact is loaded into an Extism container whose host
/// functions may write generated files only below `working_directory`.
///
/// The handshake checks the guest against its installed metadata. When the
/// metadata records a frontend or backend capability, those members are
/// locked. When it records neither, the guest is checked as a schema v1
/// extension, which may generate without a typed backend capability.
///
/// # Example
///
/// Start an installed guest, run the MEP handshake over its checked
/// connection, generate with it, and close the session in order.
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use morphir_distribution::VerifiedExtensionArtifact;
/// use morphir_extension_sdk::GenerateRequest;
/// use morphir_extension_sdk::protocol::{PeerInfo, PeerKind};
/// use morphir_host::{HostConfig, Session};
/// use morphir_host_native::activate;
/// use std::path::Path;
///
/// # let artifact: VerifiedExtensionArtifact = todo!();
/// let working_directory = Path::new("/var/morphir/generated");
///
/// // Start the guest from its verified artifact.
/// let guest = activate(artifact, working_directory).await?;
///
/// // Run the MEP handshake over the guest's checked connection.
/// let config = HostConfig::new(PeerInfo {
///     kind: PeerKind::Unspecified,
///     name: "example-host".into(),
///     version: "1.0.0".into(),
/// });
/// let mut session = Session::open(guest.connection, &config).await?;
///
/// // Make one call with the open session.
/// let request = GenerateRequest {
///     ir: serde_json::json!({}),
///     target: "example-target".into(),
///     options: Default::default(),
/// };
/// let result = session.generate(request).await?;
/// println!("generated {} artifact(s)", result.artifacts.len());
///
/// // Close the session in order, even on the happy path.
/// session.close().await?;
/// # Ok(())
/// # }
/// ```
pub async fn activate(
    artifact: VerifiedExtensionArtifact,
    working_directory: &Path,
) -> Result<ActivatedGuest, HostError> {
    let (channel, expectation): (Box<dyn Channel>, ExpectedExtension) = match artifact {
        VerifiedExtensionArtifact::Process(process) => {
            let capabilities = process.extension_capabilities();
            let persisted = if process.supplied_claims().is_some() {
                PersistedExtensionCapabilities::from_claims(
                    capabilities.frontend,
                    capabilities.backend,
                )
            } else {
                PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend)
            };
            let launch = if !persisted.is_empty() {
                ProcessLaunch::from_verified_bytes_with_persisted_capabilities_in(
                    process.extension_info().clone(),
                    persisted,
                    process.filename(),
                    process.bytes(),
                    process.staging_directory(),
                    working_directory,
                )
            } else {
                ProcessLaunch::from_legacy_verified_bytes_in(
                    process.extension_info().clone(),
                    process.filename(),
                    process.bytes(),
                    process.staging_directory(),
                    working_directory,
                )
            };
            let launch = process
                .args()
                .iter()
                .fold(launch, |launch, argument| launch.arg(argument));
            let channel = ProcessChannel::spawn(launch).await?;
            let expectation = channel.expectation();
            (Box::new(channel), expectation)
        }
        VerifiedExtensionArtifact::Wasm(wasm) => {
            let info = wasm.extension_info().clone();
            let capabilities = wasm.extension_capabilities();
            let persisted = if wasm.supplied_claims().is_some() {
                PersistedExtensionCapabilities::from_claims(
                    capabilities.frontend,
                    capabilities.backend,
                )
            } else {
                PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend)
            };
            let container = ExtensionContainer::from_bytes_async(
                info.id.clone(),
                wasm.into_bytes(),
                wasm_host_functions(working_directory),
            )
            .await?;
            let expected = if !persisted.is_empty() {
                ExpectedExtension::discovered_with_persisted_capabilities(info, persisted)
            } else {
                ExpectedExtension::legacy_discovered(info)
            };
            let channel = ExtismChannel::new(container, expected);
            let expectation = channel.expectation();
            (Box::new(channel), expectation)
        }
    };
    let id = expectation.id().to_owned();
    let connection = JsonRpcConnection::new(channel, ExpectedChecks::new(expectation.clone()));
    Ok(ActivatedGuest {
        connection: CheckedConnection::new(connection),
        id,
        expectation,
    })
}

/// The host functions an installed WebAssembly guest gets.
///
/// The guest may write generated files only below `working_directory`, and
/// it has no output directory of its own: the host publishes artifacts.
fn wasm_host_functions(working_directory: &Path) -> MorphirHostFunctions {
    MorphirHostFunctions::for_restricted_generation(working_directory.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::wasm_host_functions;

    #[test]
    fn installed_wasm_activation_uses_restricted_generation_host_policy() {
        let workspace = tempfile::tempdir().unwrap();

        let host = wasm_host_functions(workspace.path());

        assert!(host.workspace_info().output_dir.is_empty());
    }
}
