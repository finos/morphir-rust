//! Driver for cache namespace inventory acceptance scenarios.

use morphir_common::cache_maintenance::{
    CacheEntry, CacheEntryState, CacheInventoryLimits, CacheNamespace, inventory_cache_namespace,
};
use morphir_common::home::MorphirHome;
use tempfile::TempDir;

#[derive(Debug, Default)]
pub struct CacheInventoryDriver {
    home_root: Option<TempDir>,
    home: Option<MorphirHome>,
    namespace: Option<CacheNamespace>,
    result: Option<Result<Vec<CacheEntry>, String>>,
    outside: Option<TempDir>,
    link_supported: bool,
}

impl CacheInventoryDriver {
    pub fn given_classified_entries(&mut self) {
        let (root, home) = a_morphir_home();
        std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
        std::fs::write(home.downloads_cache_dir().join("owned.pkg"), b"owned").unwrap();
        std::fs::write(home.downloads_cache_dir().join("leased.pkg"), b"leased").unwrap();
        std::fs::write(home.downloads_cache_dir().join("unknown.pkg"), b"unknown").unwrap();
        self.namespace = Some(
            CacheNamespace::new("downloads")
                .unwrap()
                .with_disposable("owned.pkg", 10)
                .unwrap()
                .with_lease("leased.pkg", 20)
                .unwrap(),
        );
        self.home_root = Some(root);
        self.home = Some(home);
    }

    pub fn given_entry_budget_overflow(&mut self) {
        let (root, home) = a_morphir_home();
        std::fs::create_dir_all(home.indexes_cache_dir()).unwrap();
        std::fs::write(home.indexes_cache_dir().join("first"), b"1").unwrap();
        std::fs::write(home.indexes_cache_dir().join("second"), b"2").unwrap();
        self.namespace = Some(CacheNamespace::new("indexes").unwrap());
        self.home_root = Some(root);
        self.home = Some(home);
    }

    pub fn given_link_like_namespace_root(&mut self) {
        let (root, home) = a_morphir_home();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("keep"), b"outside").unwrap();
        std::fs::create_dir_all(home.cache_dir()).unwrap();
        self.link_supported =
            create_directory_link(outside.path(), &home.downloads_cache_dir()).is_ok();
        self.namespace = Some(CacheNamespace::new("downloads").unwrap());
        self.home_root = Some(root);
        self.home = Some(home);
        self.outside = Some(outside);
    }

    pub fn when_inventorying(&mut self) {
        self.inventory_with(CacheInventoryLimits::default());
    }

    pub fn when_inventorying_with_one_entry(&mut self) {
        self.inventory_with(CacheInventoryLimits::new(1, 8).unwrap());
    }

    pub fn assert_disposable_entry(&self) {
        let entry = self.entry("owned.pkg");
        assert_eq!(entry.bytes(), 5);
        assert_eq!(entry.state(), CacheEntryState::Disposable { last_used: 10 });
    }

    pub fn assert_leased_entry(&self) {
        assert_eq!(
            self.entry("leased.pkg").state(),
            CacheEntryState::ActiveLease { last_used: 20 }
        );
    }

    pub fn assert_unknown_entry(&self) {
        assert_eq!(
            self.entry("unknown.pkg").state(),
            CacheEntryState::Unclassified
        );
    }

    pub fn assert_entry_limit_failure(&self) {
        assert!(self.error().contains("entry limit"));
    }

    pub fn assert_link_refused(&self) {
        if self.link_supported {
            assert!(self.error().contains("link-like namespace root"));
        }
    }

    pub fn assert_outside_unchanged(&self) {
        assert_eq!(
            std::fs::read(self.outside.as_ref().unwrap().path().join("keep")).unwrap(),
            b"outside"
        );
    }

    fn inventory_with(&mut self, limits: CacheInventoryLimits) {
        if self.outside.is_some() && !self.link_supported {
            return;
        }
        self.result = Some(
            inventory_cache_namespace(
                self.home.as_ref().unwrap(),
                self.namespace.as_ref().unwrap(),
                limits,
            )
            .map_err(|error| error.to_string()),
        );
    }

    fn entry(&self, path: &str) -> &CacheEntry {
        self.result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .iter()
            .find(|entry| entry.path() == path)
            .unwrap()
    }

    fn error(&self) -> &str {
        self.result.as_ref().unwrap().as_ref().unwrap_err()
    }
}

fn a_morphir_home() -> (TempDir, MorphirHome) {
    let root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
    (root, home)
}

#[cfg(unix)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
