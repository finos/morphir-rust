//! Runtime-neutral activation for verified extension artifacts.

use crate::Result;
use crate::extensions::host_functions::MorphirHostFunctions;
use crate::extensions::session::ExtismTransport;
use crate::extensions::{
    ExtensionContainer, Loaded, MepTransport, PersistedExtensionCapabilities, ProcessLaunch,
    Session, SpawnedProcessTransport,
};
use morphir_distribution::VerifiedExtensionArtifact;
use std::path::Path;

/// A runtime-erased transport shared by native and WebAssembly extensions.
pub type BoxedMepTransport = Box<dyn MepTransport>;

/// Activate a verified artifact without starting MEP negotiation.
pub async fn activate_transport(
    artifact: VerifiedExtensionArtifact,
    working_directory: &Path,
) -> Result<Session<BoxedMepTransport, Loaded>> {
    let transport: BoxedMepTransport = match artifact {
        VerifiedExtensionArtifact::Process(process) => {
            let capabilities = process.extension_capabilities();
            let persisted_capabilities =
                PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend);
            let launch = if !persisted_capabilities.is_empty() {
                ProcessLaunch::from_verified_bytes_with_persisted_capabilities_in(
                    process.extension_info().clone(),
                    persisted_capabilities,
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
            Box::new(SpawnedProcessTransport::spawn(launch).await?)
        }
        VerifiedExtensionArtifact::Wasm(wasm) => {
            let extension_info = wasm.extension_info().clone();
            let capabilities = wasm.extension_capabilities();
            let persisted_capabilities =
                PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend);
            let container = ExtensionContainer::from_bytes_async(
                extension_info.id.clone(),
                wasm.into_bytes(),
                wasm_host_functions(working_directory),
            )
            .await?;
            if !persisted_capabilities.is_empty() {
                Box::new(ExtismTransport::new_with_persisted_capabilities(
                    container,
                    extension_info,
                    persisted_capabilities,
                ))
            } else {
                Box::new(ExtismTransport::new_with_legacy_expected_extension(
                    container,
                    extension_info,
                ))
            }
        }
    };
    Ok(Session::loaded(transport))
}

fn wasm_host_functions(working_directory: &Path) -> MorphirHostFunctions {
    MorphirHostFunctions::for_restricted_generation(working_directory.to_path_buf())
}

#[cfg(test)]
mod tests;
