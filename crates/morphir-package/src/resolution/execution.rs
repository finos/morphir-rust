use super::model::{ReleaseId, ReleaseRecord, Requirement, ResolutionExecutionError};

/// Aggregate allocations and traversal performed by one resolution request.
pub(super) const DEFAULT_WORK_LIMIT: usize = 100_000;

pub(super) struct Budget {
    limit: usize,
    remaining: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            limit: DEFAULT_WORK_LIMIT,
            remaining: DEFAULT_WORK_LIMIT,
        }
    }
}

impl Budget {
    #[cfg(test)]
    pub(super) fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            remaining: limit,
        }
    }

    pub(super) fn charge(
        &mut self,
        units: usize,
        activity: &str,
    ) -> Result<(), ResolutionExecutionError> {
        let Some(remaining) = self.remaining.checked_sub(units) else {
            return Err(ResolutionExecutionError::new(format!(
                "{activity} exceeds the {}-unit aggregate execution budget",
                self.limit
            )));
        };
        self.remaining = remaining;
        Ok(())
    }
}

pub(super) fn release_id_clone_weight(release: &ReleaseId) -> usize {
    // Package path, version spelling, and the version's three exact-decimal components.
    let _ = release;
    5
}

pub(super) fn requirement_clone_weight(requirement: &Requirement) -> usize {
    // IR name, package path, and both version bounds.
    let _ = requirement;
    2usize.saturating_add(2usize.saturating_mul(4))
}

pub(super) fn release_record_clone_weight(record: &ReleaseRecord) -> usize {
    release_id_clone_weight(&record.release)
        .saturating_add(3) // IR name and two digests.
        .saturating_add(
            record
                .dependencies
                .iter()
                .fold(0usize, |weight, dependency| {
                    weight.saturating_add(requirement_clone_weight(dependency))
                }),
        )
}
