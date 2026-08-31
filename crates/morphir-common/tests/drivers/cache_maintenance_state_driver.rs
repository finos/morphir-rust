//! Test driver for coordinated automatic cache-maintenance scenarios.

use morphir_common::cache_maintenance::{
    AutomaticCacheMaintenanceTransaction, CacheExecutionLimits, CacheExecutionReport,
    CacheInventoryLimits, CacheNamespace, CachePolicy, CleanupMode, CleanupPlan,
    inventory_cache_namespace, load_cache_maintenance_state, plan_cache_cleanup,
};
use morphir_common::home::MorphirHome;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

const COMPLETED_AT: u64 = 1_735_689_600;

#[derive(Debug, Default)]
pub struct CacheMaintenanceStateDriver {
    home_root: Option<TempDir>,
    home: Option<MorphirHome>,
    cache_path: Option<PathBuf>,
    plan: Option<CleanupPlan>,
    ownership: Vec<CacheNamespace>,
    result: Option<Result<CacheExecutionReport, String>>,
}

impl CacheMaintenanceStateDriver {
    pub fn given_registered_stale_entry(&mut self) {
        let home_root = TempDir::new().unwrap();
        let home = MorphirHome::resolve_from(Some(home_root.path().as_os_str()), None).unwrap();
        let cache_path = home.downloads_cache_dir().join("desktop.pkg");
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, b"stale desktop package").unwrap();
        let namespace = CacheNamespace::new("downloads")
            .unwrap()
            .with_disposable("desktop.pkg", COMPLETED_AT - 10)
            .unwrap();
        let inventory =
            inventory_cache_namespace(&home, &namespace, CacheInventoryLimits::default()).unwrap();
        let plan = plan_cache_cleanup(
            inventory,
            CachePolicy::new(Duration::from_secs(1), 0),
            COMPLETED_AT,
            CleanupMode::Policy,
        )
        .unwrap();

        self.home_root = Some(home_root);
        self.home = Some(home);
        self.cache_path = Some(cache_path);
        self.plan = Some(plan);
        self.ownership = vec![namespace];
    }

    pub fn when_running_transaction(&mut self) {
        let result = (|| {
            let home = self.home.as_ref().unwrap();
            let transaction = AutomaticCacheMaintenanceTransaction::begin(home)
                .map_err(|error| error.to_string())?;
            let completed = transaction.state().clone().completed(COMPLETED_AT);
            let report = transaction
                .execute_cleanup(
                    self.plan.as_ref().unwrap(),
                    &self.ownership,
                    CacheInventoryLimits::default(),
                    CacheExecutionLimits::new(1, 1024).unwrap(),
                )
                .map_err(|error| error.to_string())?;
            transaction
                .finish(completed)
                .map_err(|error| error.to_string())?;
            Ok(report)
        })();
        self.result = Some(result);
    }

    pub fn assert_registered_entry_removed(&self) {
        self.report();
        assert!(!self.cache_path.as_ref().unwrap().exists());
    }

    pub fn assert_success_timestamp_durable(&self) {
        self.report();
        assert_eq!(
            load_cache_maintenance_state(self.home.as_ref().unwrap())
                .unwrap()
                .last_successful_automatic_run(),
            Some(COMPLETED_AT)
        );
    }

    fn report(&self) -> &CacheExecutionReport {
        self.result.as_ref().unwrap().as_ref().unwrap()
    }
}
