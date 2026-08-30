//! Driver for bounded cache cleanup execution acceptance scenarios.

use morphir_common::cache_maintenance::{
    CacheExecutionDisposition, CacheExecutionLimits, CacheExecutionReport, CacheInventoryLimits,
    CacheNamespace, CachePolicy, CleanupMode, CleanupPlan, execute_cache_cleanup,
    inventory_cache_namespace, plan_cache_cleanup,
};
use morphir_common::home::MorphirHome;
use std::time::Duration;
use tempfile::TempDir;

const INTERRUPTED_RUN: &str = "00000000000000000000000000000000";

#[derive(Debug, Default)]
pub struct CacheExecutionDriver {
    home_root: Option<TempDir>,
    home: Option<MorphirHome>,
    plan: Option<CleanupPlan>,
    ownership: Vec<CacheNamespace>,
    limits: Option<CacheExecutionLimits>,
    result: Option<Result<CacheExecutionReport, String>>,
}

impl CacheExecutionDriver {
    pub fn given_owned_and_unknown(&mut self) {
        let (root, home) = a_morphir_home();
        std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
        std::fs::write(home.downloads_cache_dir().join("owned.pkg"), b"owned").unwrap();
        std::fs::write(home.downloads_cache_dir().join("unknown.pkg"), b"unknown").unwrap();
        let namespace = CacheNamespace::new("downloads")
            .unwrap()
            .with_disposable("owned.pkg", 1)
            .unwrap();
        self.configure(root, home, vec![namespace], 10);
    }

    pub fn given_late_lease(&mut self) {
        let (root, home) = a_morphir_home();
        std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
        std::fs::write(
            home.downloads_cache_dir().join("candidate.pkg"),
            b"candidate",
        )
        .unwrap();
        let disposable = CacheNamespace::new("downloads")
            .unwrap()
            .with_disposable("candidate.pkg", 1)
            .unwrap();
        let plan = cleanup_plan(&home, &disposable);
        let leased = CacheNamespace::new("downloads")
            .unwrap()
            .with_lease("candidate.pkg", 2)
            .unwrap();
        self.home_root = Some(root);
        self.home = Some(home);
        self.plan = Some(plan);
        self.ownership = vec![leased];
        self.limits = Some(CacheExecutionLimits::new(10, 1_000).unwrap());
    }

    pub fn given_bounded_candidates(&mut self) {
        let (root, home) = a_morphir_home();
        std::fs::create_dir_all(home.downloads_cache_dir()).unwrap();
        std::fs::write(home.downloads_cache_dir().join("first.pkg"), b"first").unwrap();
        std::fs::write(home.downloads_cache_dir().join("second.pkg"), b"second").unwrap();
        let namespace = CacheNamespace::new("downloads")
            .unwrap()
            .with_disposable("first.pkg", 1)
            .unwrap()
            .with_disposable("second.pkg", 2)
            .unwrap();
        self.configure(root, home, vec![namespace], 1);
    }

    pub fn given_interrupted_trash(&mut self) {
        let (root, home) = a_morphir_home();
        let run = home.maintenance_trash_dir().join(INTERRUPTED_RUN);
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("verified-00000000"), b"trash").unwrap();
        self.home_root = Some(root);
        self.home = Some(home);
        self.plan = Some(
            plan_cache_cleanup(
                Vec::new(),
                CachePolicy::new(Duration::ZERO, 0),
                100,
                CleanupMode::All,
            )
            .unwrap(),
        );
        self.ownership = Vec::new();
        self.limits = Some(CacheExecutionLimits::new(10, 1_000).unwrap());
    }

    pub fn when_executing(&mut self) {
        self.result = Some(
            execute_cache_cleanup(
                self.home.as_ref().unwrap(),
                self.plan.as_ref().unwrap(),
                &self.ownership,
                CacheInventoryLimits::default(),
                self.limits.unwrap(),
            )
            .map_err(|error| error.to_string()),
        );
    }

    pub fn assert_owned_removed(&self) {
        assert_eq!(
            self.report().items()[0].disposition(),
            CacheExecutionDisposition::Removed
        );
        assert!(!self.cache_path("owned.pkg").exists());
    }

    pub fn assert_unknown_remains(&self) {
        assert_eq!(
            std::fs::read(self.cache_path("unknown.pkg")).unwrap(),
            b"unknown"
        );
    }

    pub fn assert_late_lease_deferred(&self) {
        assert_eq!(
            self.report().items()[0].disposition(),
            CacheExecutionDisposition::ActiveLease
        );
    }

    pub fn assert_leased_remains(&self) {
        assert_eq!(
            std::fs::read(self.cache_path("candidate.pkg")).unwrap(),
            b"candidate"
        );
    }

    pub fn assert_one_removed_one_deferred(&self) {
        assert_eq!(
            self.report()
                .items()
                .iter()
                .map(|item| item.disposition())
                .collect::<Vec<_>>(),
            [
                CacheExecutionDisposition::Removed,
                CacheExecutionDisposition::DeferredLimit
            ]
        );
        assert_eq!(
            ["first.pkg", "second.pkg"]
                .into_iter()
                .filter(|path| self.cache_path(path).exists())
                .count(),
            1
        );
    }

    pub fn assert_interrupted_trash_removed(&self) {
        self.report();
        assert!(
            !self
                .home
                .as_ref()
                .unwrap()
                .maintenance_trash_dir()
                .join(INTERRUPTED_RUN)
                .exists()
        );
    }

    fn configure(
        &mut self,
        root: TempDir,
        home: MorphirHome,
        ownership: Vec<CacheNamespace>,
        max_removals: usize,
    ) {
        let plan = cleanup_plan(&home, &ownership[0]);
        self.home_root = Some(root);
        self.home = Some(home);
        self.plan = Some(plan);
        self.ownership = ownership;
        self.limits = Some(CacheExecutionLimits::new(max_removals, 1_000).unwrap());
    }

    fn report(&self) -> &CacheExecutionReport {
        self.result.as_ref().unwrap().as_ref().unwrap()
    }

    fn cache_path(&self, name: &str) -> std::path::PathBuf {
        self.home.as_ref().unwrap().downloads_cache_dir().join(name)
    }
}

fn cleanup_plan(home: &MorphirHome, namespace: &CacheNamespace) -> CleanupPlan {
    let inventory =
        inventory_cache_namespace(home, namespace, CacheInventoryLimits::default()).unwrap();
    plan_cache_cleanup(
        inventory,
        CachePolicy::new(Duration::ZERO, 0),
        100,
        CleanupMode::All,
    )
    .unwrap()
}

fn a_morphir_home() -> (TempDir, MorphirHome) {
    let root = TempDir::new().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().as_os_str()), None).unwrap();
    (root, home)
}
