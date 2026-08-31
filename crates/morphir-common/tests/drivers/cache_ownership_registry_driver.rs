//! Test driver for trusted cache-ownership scenarios.

use morphir_common::cache_maintenance::{
    CacheEntry, CacheEntryState, CacheExecutionLimits, CacheInventoryLimits,
    CacheMaintenanceSession, CacheOwnershipMutationGuard, CachePolicy, CleanupMode,
    plan_cache_cleanup,
};
use morphir_common::home::MorphirHome;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

const NOW: u64 = 100;

#[derive(Debug, Default)]
pub struct CacheOwnershipRegistryDriver {
    home_root: Option<TempDir>,
    home: Option<MorphirHome>,
    registered: Option<PathBuf>,
    unknown: Option<PathBuf>,
    released: Option<PathBuf>,
    inventory: Vec<CacheEntry>,
    cleanup_error: Option<String>,
}

impl CacheOwnershipRegistryDriver {
    pub fn given_registered_and_unknown_files(&mut self) {
        let (home_root, home) = new_home();
        let registered = home.downloads_cache_dir().join("registered.pkg");
        let unknown = home.downloads_cache_dir().join("unknown.pkg");
        std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
        std::fs::write(&registered, b"registered").unwrap();
        std::fs::write(&unknown, b"unknown").unwrap();
        register(&home, "downloads", "registered.pkg", 1);

        self.home_root = Some(home_root);
        self.home = Some(home);
        self.registered = Some(registered);
        self.unknown = Some(unknown);
    }

    pub fn given_released_cache_file(&mut self) {
        let (home_root, home) = new_home();
        let released = home.downloads_cache_dir().join("released.pkg");
        std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
        std::fs::write(&released, b"released").unwrap();
        register(&home, "downloads", "released.pkg", 1);
        assert!(
            CacheOwnershipMutationGuard::begin(&home, "downloads", "released.pkg")
                .unwrap()
                .finish_unowned()
        );

        self.home_root = Some(home_root);
        self.home = Some(home);
        self.released = Some(released);
    }

    pub fn when_running_guarded_cleanup(&mut self) {
        let result = (|| {
            let session = CacheMaintenanceSession::begin(self.home.as_ref().unwrap())
                .map_err(|error| error.to_string())?;
            let inventory = session
                .inventory(&["downloads"], CacheInventoryLimits::default())
                .map_err(|error| error.to_string())?;
            let plan = plan_cache_cleanup(
                inventory.clone(),
                CachePolicy::new(Duration::from_secs(1), 0),
                NOW,
                CleanupMode::All,
            )
            .map_err(|error| error.to_string())?;
            session
                .execute_cleanup(
                    &plan,
                    CacheInventoryLimits::default(),
                    CacheExecutionLimits::new(10, 1024).unwrap(),
                )
                .map_err(|error| error.to_string())?;
            self.inventory = inventory;
            Ok::<(), String>(())
        })();
        self.cleanup_error = result.err();
    }

    pub fn assert_registered_file_removed(&self) {
        self.assert_cleanup_succeeded();
        assert!(!self.registered.as_ref().unwrap().exists());
    }

    pub fn assert_unknown_file_remains(&self) {
        self.assert_cleanup_succeeded();
        assert!(self.unknown.as_ref().unwrap().exists());
    }

    pub fn assert_released_file_remains_unclassified(&self) {
        self.assert_cleanup_succeeded();
        assert!(self.released.as_ref().unwrap().exists());
        assert!(self.inventory.iter().any(|entry| {
            entry.path() == "released.pkg" && matches!(entry.state(), CacheEntryState::Unclassified)
        }));
    }

    fn assert_cleanup_succeeded(&self) {
        assert_eq!(self.cleanup_error, None);
    }
}

fn new_home() -> (TempDir, MorphirHome) {
    let home_root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(home_root.path().as_os_str()), None).unwrap();
    (home_root, home)
}

fn register(home: &MorphirHome, namespace: &str, path: &str, last_used: u64) {
    CacheOwnershipMutationGuard::begin(home, namespace, path)
        .unwrap()
        .finish(last_used)
        .unwrap();
}
