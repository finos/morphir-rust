//! Validated resource limits for a guest with no ambient access.

use morphir_host::HostError;
use serde::Serialize;
use std::io::{self, Write};
use std::time::Duration;

/// An opt-in legacy Extism profile with no HTTP hosts, WASI preopens or
/// inherited guest environment. Limits cover metadata discovery and every call.
///
/// This policy can only tighten the existing legacy ceilings. It cannot opt in
/// to the larger generated-artifact response profile. The caller still owns
/// artifact integrity, whole-session deadlines and process isolation/cleanup.
///
/// ```
/// use morphir_host_native::extism::RestrictedExtismPolicy;
/// use std::time::Duration;
/// let policy = RestrictedExtismPolicy::new(
///     256 * 1024 * 1024, 1_000_000_000, Duration::from_secs(30),
///     4 * 1024 * 1024, 16 * 1024 * 1024,
/// )?;
/// # Ok::<(), morphir_host::HostError>(())
/// ```
#[derive(Clone, Copy, Debug)]
pub struct RestrictedExtismPolicy {
    pub(super) memory_bytes: u64,
    pub(super) fuel_per_call: u64,
    pub(super) call_timeout: Duration,
    pub(super) request_bytes: u64,
    pub(super) response_bytes: u64,
}

impl RestrictedExtismPolicy {
    /// Validate positive finite limits within 256 MiB memory, one billion fuel,
    /// 1 millisecond to 30 seconds per call, and 64 MiB for each request or response.
    /// Memory must be a whole number of 64 KiB WebAssembly pages.
    pub fn new(
        memory_bytes: u64,
        fuel_per_call: u64,
        call_timeout: Duration,
        request_bytes: u64,
        response_bytes: u64,
    ) -> Result<Self, HostError> {
        let invalid =
            |resource| HostError::Invalid(format!("Invalid restricted Extism {resource} limit"));
        if memory_bytes == 0
            || memory_bytes > 256 * 1024 * 1024
            || !memory_bytes.is_multiple_of(64 * 1024)
        {
            return Err(invalid("memory"));
        }
        if fuel_per_call == 0 || fuel_per_call > 1_000_000_000 {
            return Err(invalid("fuel"));
        }
        if call_timeout < Duration::from_millis(1) || call_timeout > Duration::from_secs(30) {
            return Err(invalid("timeout"));
        }
        if request_bytes == 0 || request_bytes > 64 * 1024 * 1024 {
            return Err(invalid("request"));
        }
        if response_bytes == 0 || response_bytes > 64 * 1024 * 1024 {
            return Err(invalid("response"));
        }
        Ok(Self {
            memory_bytes,
            fuel_per_call,
            call_timeout,
            request_bytes,
            response_bytes,
        })
    }

    pub(super) fn check_request(self, bytes: usize) -> Result<(), HostError> {
        check_bytes("request", self.request_bytes, bytes)
    }

    pub(super) fn check_response(self, bytes: usize) -> Result<(), HostError> {
        check_bytes("response", self.response_bytes, bytes)
    }

    pub(super) fn encode_request(self, value: &impl Serialize) -> Result<Vec<u8>, HostError> {
        let mut output = BoundedRequest {
            bytes: Vec::new(),
            limit: self.request_bytes as usize,
        };
        serde_json::to_writer(&mut output, value)?;
        Ok(output.bytes)
    }
}

struct BoundedRequest {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedRequest {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Extension request exceeds the {} byte limit", self.limit,),
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn check_bytes(direction: &str, limit: u64, actual: usize) -> Result<(), HostError> {
    if actual as u64 > limit {
        Err(HostError::Invalid(format!(
            "Extension {direction} exceeds the {limit} byte limit"
        )))
    } else {
        Ok(())
    }
}
