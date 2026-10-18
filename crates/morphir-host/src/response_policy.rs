//! Explicit response budgets for generated-artifact invocations.

use crate::HostError;
use morphir_extension_sdk::protocol::methods;

const LEGACY_RESPONSE_BYTES: u32 = 64 * 1024 * 1024;
// 64 MiB decoded text can require six JSON bytes per byte, plus 1 MiB
// for the SDK envelope, file metadata and diagnostics. Binary base64 is smaller.
const ARTIFACT_RESPONSE_BYTES: u32 = 385 * 1024 * 1024;

/// The response policy selected when opening a guest.
///
/// This is host-side resource policy, not a new SDK contract or a capability
/// grant. Request bounds never change. Keep pools separated by this value so
/// a warm artifact guest cannot be reused under the legacy policy.
///
/// ```
/// use morphir_host::GuestResponsePolicy;
/// let policy = GuestResponsePolicy::GeneratedArtifacts;
/// assert_eq!(policy.response_limit("morphir.backend.generate").bytes(), 385 * 1024 * 1024);
/// assert_eq!(policy.response_limit("morphir.frontend.compile").bytes(), 64 * 1024 * 1024);
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestResponsePolicy {
    /// Preserve the existing 64 MiB response limit for every method.
    #[default]
    Legacy,
    /// Permit bounded artifact responses only for `morphir.backend.generate`.
    GeneratedArtifacts,
}

impl GuestResponsePolicy {
    /// The exact bound for the response to `method`.
    pub fn response_limit(self, method: &str) -> ResponseByteLimit {
        let bytes = match (self, method) {
            (Self::GeneratedArtifacts, methods::GENERATE) => ARTIFACT_RESPONSE_BYTES,
            _ => LEGACY_RESPONSE_BYTES,
        };
        ResponseByteLimit(bytes)
    }
}

/// A finite response bound produced by a supported host policy.
///
/// There is no unchecked numeric constructor. Transports check the declared
/// length before allocating a body, or the borrowed output before copying it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseByteLimit(u32);

impl ResponseByteLimit {
    /// The maximum encoded response length, in bytes.
    pub fn bytes(self) -> u32 {
        self.0
    }

    /// Refuse a response whose encoded byte length exceeds this bound.
    pub fn check_len(self, bytes: usize) -> Result<(), HostError> {
        if bytes > self.0 as usize {
            return Err(HostError::Invalid(format!(
                "Extension response exceeds the {} byte limit",
                self.0
            )));
        }
        Ok(())
    }
}
