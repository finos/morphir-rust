//! Description of a native extension process before it is spawned.

use morphir_extension_sdk::{ExtensionCapabilities, ExtensionInfo};
use morphir_host::{CapabilityExpectation, ExpectedExtension, PersistedExtensionCapabilities};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A native extension command with an explicit identity and working directory.
#[derive(Debug, Clone)]
pub struct ProcessLaunch {
    extension_id: String,
    discovered: Option<ExtensionInfo>,
    capabilities: Option<CapabilityExpectation>,
    allows_legacy_backend: bool,
    program: ProcessProgram,
    args: Vec<OsString>,
    working_directory: PathBuf,
    environment: Vec<(OsString, OsString)>,
    request_timeout: Duration,
}

/// Where the child process's executable bytes come from.
///
/// Hidden from public docs: callers build a launch through `ProcessLaunch`'s
/// constructors, not by naming this type directly.
#[derive(Debug, Clone)]
#[doc(hidden)]
pub enum ProcessProgram {
    /// An executable already present on disk.
    Path(PathBuf),
    /// Verified bytes staged to disk before the process starts.
    VerifiedBytes {
        /// The single filename the staged executable is written as.
        filename: OsString,
        /// The executable's verified content.
        bytes: Arc<[u8]>,
        /// Where to stage the executable, when the caller manages the directory.
        staging_directory: Option<PathBuf>,
    },
}

impl ProcessLaunch {
    /// Define a process launch without inheriting the host environment.
    pub fn new(
        extension_id: impl Into<String>,
        program: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            discovered: None,
            capabilities: None,
            allows_legacy_backend: false,
            program: ProcessProgram::Path(program.into()),
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch with exact discovery metadata.
    ///
    /// Typestate initialization requires the child to reproduce this identity,
    /// name, version, and capability set. Use [`Self::new`] for explicit
    /// development commands that have only a configured identity.
    pub fn from_discovered(
        discovered: ExtensionInfo,
        program: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: None,
            allows_legacy_backend: false,
            program: ProcessProgram::Path(program.into()),
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch with exact discovery capabilities.
    pub fn from_discovered_capabilities(
        discovered: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        program: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Exact(capabilities)),
            allows_legacy_backend: false,
            program: ProcessProgram::Path(program.into()),
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch from immutable executable bytes.
    pub fn from_verified_bytes(
        discovered: ExtensionInfo,
        filename: &OsStr,
        bytes: &[u8],
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: None,
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: None,
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch staged below an explicit executable directory.
    pub fn from_verified_bytes_in(
        discovered: ExtensionInfo,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: None,
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: Some(staging_directory.into()),
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified process launch from immutable bytes and capabilities.
    pub fn from_verified_bytes_with_capabilities(
        discovered: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        filename: &OsStr,
        bytes: &[u8],
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Exact(capabilities)),
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: None,
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a capability-locked verified launch below an executable directory.
    pub fn from_verified_bytes_with_capabilities_in(
        discovered: ExtensionInfo,
        capabilities: ExtensionCapabilities,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Exact(capabilities)),
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: Some(staging_directory.into()),
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified launch with persisted capability members below an executable directory.
    pub fn from_verified_bytes_with_persisted_capabilities_in(
        discovered: ExtensionInfo,
        capabilities: PersistedExtensionCapabilities,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Persisted(capabilities)),
            allows_legacy_backend: false,
            program: ProcessProgram::VerifiedBytes {
                filename: filename.to_os_string(),
                bytes: Arc::from(bytes),
                staging_directory: Some(staging_directory.into()),
            },
            args: Vec::new(),
            working_directory: working_directory.into(),
            environment: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Define a verified schema-v1 process launch with legacy backend behavior.
    #[doc(hidden)]
    pub fn from_legacy_verified_bytes_in(
        discovered: ExtensionInfo,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        let mut launch = Self::from_verified_bytes_in(
            discovered,
            filename,
            bytes,
            staging_directory,
            working_directory,
        );
        launch.allows_legacy_backend = true;
        launch
    }

    /// Append one process argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add one environment variable to the otherwise empty child environment.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.push((key.into(), value.into()));
        self
    }

    /// Set the timeout applied to each request and to process shutdown.
    pub fn request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// Return the extension identity this launch expects to start.
    pub fn extension_id(&self) -> &str {
        &self.extension_id
    }

    /// Return the process arguments, in order.
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// Return the child process's working directory.
    pub fn working_directory(&self) -> &std::path::Path {
        &self.working_directory
    }

    /// Return the child process's environment variables.
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    /// Return the executable this launch starts.
    #[doc(hidden)]
    pub fn program(&self) -> &ProcessProgram {
        &self.program
    }

    /// Return the timeout applied to each request and to process shutdown.
    ///
    /// Named apart from the [`Self::request_timeout`] builder, which already
    /// owns that name as a consuming setter.
    pub fn configured_request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// Return what the host should expect of the extension this launch starts.
    ///
    /// This is the identity and capability lock negotiation checks against:
    /// exact discovery metadata and capabilities when both are known, the
    /// schema v1 legacy exemption when only discovery metadata is known and
    /// legacy behavior is allowed, discovery metadata alone otherwise, or
    /// only the configured identifier when nothing was discovered.
    pub fn expectation(&self) -> ExpectedExtension {
        match (&self.discovered, &self.capabilities) {
            (Some(discovered), Some(capabilities)) => {
                ExpectedExtension::discovered_with_expectation(
                    discovered.clone(),
                    capabilities.clone(),
                )
            }
            (Some(discovered), None) if self.allows_legacy_backend => {
                ExpectedExtension::legacy_discovered(discovered.clone())
            }
            (Some(discovered), None) => ExpectedExtension::discovered(discovered.clone()),
            (None, _) => ExpectedExtension::identified(self.extension_id.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_extension_sdk::ExtensionType;

    #[test]
    fn discovered_process_launch_retains_exact_negotiation_metadata() {
        let discovered = ExtensionInfo {
            id: "morphir-elm".into(),
            name: "Morphir Elm".into(),
            version: "3.2.1".into(),
            types: vec![ExtensionType::Frontend],
            ..ExtensionInfo::default()
        };
        let launch = ProcessLaunch::from_discovered(
            discovered.clone(),
            "/verified/morphir-elm",
            "/workspace",
        );

        assert_eq!(launch.extension_id(), discovered.id);
        let expectation = launch.expectation();
        assert_eq!(expectation.id(), discovered.id);
        let retained = expectation
            .extension_info()
            .expect("verified launches should retain discovery metadata");
        assert_eq!(retained.name, discovered.name);
        assert_eq!(retained.version, discovered.version);
        assert_eq!(retained.types, discovered.types);
    }
}
