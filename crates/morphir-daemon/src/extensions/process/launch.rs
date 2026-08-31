use super::*;

/// A native extension command with an explicit identity and working directory.
#[derive(Debug, Clone)]
pub struct ProcessLaunch {
    pub(super) extension_id: String,
    pub(super) discovered: Option<ExtensionInfo>,
    pub(super) capabilities: Option<CapabilityExpectation>,
    pub(super) allows_legacy_backend: bool,
    pub(super) program: ProcessProgram,
    pub(super) args: Vec<OsString>,
    pub(super) working_directory: PathBuf,
    pub(super) environment: Vec<(OsString, OsString)>,
    pub(super) request_timeout: Duration,
}

#[derive(Debug, Clone)]
pub(super) enum ProcessProgram {
    Path(PathBuf),
    VerifiedBytes {
        filename: OsString,
        bytes: Arc<[u8]>,
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

    /// Define a backend-locked verified launch below an executable directory.
    pub(crate) fn from_verified_bytes_with_backend_capability_in(
        discovered: ExtensionInfo,
        backend: BackendCapability,
        filename: &OsStr,
        bytes: &[u8],
        staging_directory: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            extension_id: discovered.id.clone(),
            discovered: Some(discovered),
            capabilities: Some(CapabilityExpectation::Backend(backend)),
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
    pub(crate) fn from_legacy_verified_bytes_in(
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
}
