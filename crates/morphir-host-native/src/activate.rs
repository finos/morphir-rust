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
pub async fn activate(
    artifact: VerifiedExtensionArtifact,
    working_directory: &Path,
) -> Result<ActivatedGuest, HostError> {
    let (channel, expectation): (Box<dyn Channel>, ExpectedExtension) = match artifact {
        VerifiedExtensionArtifact::Process(process) => {
            let capabilities = process.extension_capabilities();
            let persisted =
                PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend);
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
            let persisted =
                PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend);
            let container = ExtensionContainer::from_bytes_async(
                info.id.clone(),
                wasm.into_bytes(),
                MorphirHostFunctions::for_restricted_generation(working_directory.to_path_buf()),
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
    let connection = JsonRpcConnection::new(channel, ExpectedChecks::new(expectation));
    Ok(ActivatedGuest {
        connection: CheckedConnection::new(connection),
        id,
    })
}
