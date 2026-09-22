use super::AdmissionError;

/// Remaining operation metadata capacity and exact same-role retry candidates.
///
/// Trusted recorders derive this from the serialized operation state. It is an
/// acquisition snapshot; recording must still atomically recheck the global budget.
#[derive(Debug, Clone)]
pub struct AcquisitionBudget {
    remaining: usize,
    previous: Vec<Vec<u8>>,
}

impl AcquisitionBudget {
    /// Bind remaining logical bytes to inputs already counted for this role.
    ///
    /// The recorder must supply only exact same-role inputs from this operation's
    /// protected inventory. This constructor checks bounds, not that provenance.
    ///
    /// ```
    /// use morphir_package::local_registry::tuf::AcquisitionBudget;
    /// let budget = AcquisitionBudget::new(1024, Vec::new())?;
    /// # Ok::<(), morphir_package::local_registry::tuf::AdmissionError>(())
    /// ```
    pub fn new(remaining: usize, previous: Vec<Vec<u8>>) -> Result<Self, AdmissionError> {
        if remaining > 268_435_456 {
            return Err(AdmissionError::Limit("metadata-bytes"));
        }
        if previous.iter().any(|bytes| bytes.len() > 16_777_216) {
            return Err(AdmissionError::Profile(
                "previous input exceeds metadata document bounds",
            ));
        }
        Ok(Self {
            remaining,
            previous,
        })
    }

    pub(super) fn check_append(
        &mut self,
        prefix: &[u8],
        chunk: &[u8],
    ) -> Result<(), AdmissionError> {
        let length = prefix
            .len()
            .checked_add(chunk.len())
            .ok_or(AdmissionError::Limit("metadata-bytes"))?;
        // Called once per ordered chunk before extending the collection buffer.
        // Survivors already match its entire prefix, so compare only new bytes.
        // This avoids quadratic rescanning when a transport emits tiny chunks.
        self.previous.retain(|known| {
            known
                .get(prefix.len()..)
                .is_some_and(|tail| tail.starts_with(chunk))
        });
        if length <= self.remaining || !self.previous.is_empty() {
            Ok(())
        } else {
            Err(AdmissionError::Limit("metadata-bytes"))
        }
    }

    pub(super) fn check_complete(&self, bytes: &[u8]) -> Result<(), AdmissionError> {
        // A shorter prefix can itself be valid JSON, but is a distinct logical
        // input. Only complete byte equality gives the already-counted exception.
        if bytes.len() <= self.remaining || self.previous.iter().any(|known| known == bytes) {
            Ok(())
        } else {
            Err(AdmissionError::Limit("metadata-bytes"))
        }
    }
}
