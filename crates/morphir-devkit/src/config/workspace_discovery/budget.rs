//! Request-wide accounting for portable configuration payloads.

use std::{fs, io::Read, path::Path};

use anyhow::{Context, Result, bail};
#[cfg(test)]
use morphir_workspace::FileEntry;

#[derive(Debug)]
pub(super) struct PayloadBudget {
    limit: usize,
    final_used: usize,
    transient_used: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PayloadKind {
    Final,
    Transient,
    Omit,
}

impl PayloadBudget {
    pub(super) const fn new(limit: usize) -> Self {
        Self {
            limit,
            final_used: 0,
            transient_used: 0,
        }
    }

    pub(super) const fn remaining(&self) -> usize {
        self.limit - self.final_used
    }

    pub(super) fn reserve(&mut self, bytes: usize) -> Result<()> {
        Self::reserve_counter(&mut self.final_used, self.limit, bytes)
    }

    pub(super) const fn transient_remaining(&self) -> usize {
        self.limit - self.transient_used
    }

    pub(super) fn reserve_transient(&mut self, bytes: usize) -> Result<()> {
        Self::reserve_counter(&mut self.transient_used, self.limit, bytes)
    }

    pub(super) fn release_transient(&mut self, bytes: usize) {
        self.transient_used = self
            .transient_used
            .checked_sub(bytes)
            .expect("only resident transient payloads are released");
    }

    fn reserve_counter(counter: &mut usize, limit: usize, bytes: usize) -> Result<()> {
        let Some(next) = counter.checked_add(bytes) else {
            return resource_limit(limit);
        };
        if next > limit {
            return resource_limit(limit);
        }
        *counter = next;
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn entry_bytes(entry: &FileEntry) -> usize {
    match entry {
        FileEntry::Directory | FileEntry::Symlink { .. } => 0,
        FileEntry::File { text } => text.len(),
    }
}

pub(super) fn resource_limit<T>(limit: usize) -> Result<T> {
    bail!(
        "workspace.traversal.resource-limit: confined native traversal exceeded fixed configuration bytes budget {limit}"
    )
}

pub(super) fn read_utf8_file(
    path: &Path,
    description: &str,
    payload: &mut PayloadBudget,
) -> Result<String> {
    let remaining = payload.remaining();
    let metadata = fs::metadata(path).with_context(|| {
        format!(
            "workspace.traversal.unreadable: Failed to inspect {description}: {}",
            path.display()
        )
    })?;
    if metadata.len() > remaining as u64 {
        return resource_limit(payload.limit);
    }
    let file = fs::File::open(path).with_context(|| {
        format!(
            "workspace.traversal.unreadable: Failed to read {description}: {}",
            path.display()
        )
    })?;
    let read_limit = u64::try_from(remaining.saturating_add(1)).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .with_context(|| {
            format!(
                "workspace.traversal.unreadable: Failed to read {description}: {}",
                path.display()
            )
        })?;
    payload.reserve(bytes.len())?;
    String::from_utf8(bytes).with_context(|| {
        format!(
            "workspace.traversal.unreadable: Failed to read UTF-8 {description}: {}",
            path.display()
        )
    })
}
