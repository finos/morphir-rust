use super::CacheEntry;
use same_file::Handle;

pub(crate) struct PinnedCacheEntry {
    entry: CacheEntry,
    handle: Option<Handle>,
}

impl PinnedCacheEntry {
    pub(super) fn new(entry: CacheEntry, handle: Option<Handle>) -> Self {
        Self { entry, handle }
    }

    pub(crate) fn entry(&self) -> &CacheEntry {
        &self.entry
    }

    pub(crate) fn handle(&self) -> Option<&Handle> {
        self.handle.as_ref()
    }

    pub(crate) fn into_entry(self) -> CacheEntry {
        self.entry
    }
}
