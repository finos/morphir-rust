//! Real process death with a bounded readiness handshake and unconditional child cleanup.
use super::state;
use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
pub struct ChildProbe {
    child: Child,
}
impl ChildProbe {
    pub fn start(mode: &str, root: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "provider_child", "--nocapture"])
            .env("MORPHIR_PROVIDER_CHILD", mode)
            .env("MORPHIR_PROVIDER_ROOT", root)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for line in io::BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if line.contains("PROVIDER_READY") {
                    let _ = sender.send(());
                }
            }
        });
        let probe = Self { child };
        receiver
            .recv_timeout(Duration::from_secs(15))
            .expect("child failed before readiness or exceeded deadline");
        probe
    }
    pub fn kill_and_wait(&mut self) {
        self.child.kill().unwrap();
        assert!(
            !self.child.wait().unwrap().success(),
            "child must be terminated, not exit cleanly"
        );
    }
    pub fn expect_success(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < deadline, "child exit deadline exceeded");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for ChildProbe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub fn lock_file(root: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("repository.lock"))
}
fn ready() {
    println!("PROVIDER_READY");
    io::stdout().flush().unwrap();
}
fn hold() -> ! {
    ready();
    loop {
        std::thread::park();
    }
}
pub fn run_child(mode: &str, root: &Path) {
    match mode {
        "lock" => {
            let file = lock_file(root).unwrap();
            fs2::FileExt::lock_exclusive(&file).unwrap();
            hold();
        }
        "before-commit" | "after-commit" => {
            let mut store = state::Store::open(&root.join("state.sqlite")).unwrap();
            store.commit_marker().unwrap();
            let transaction = store
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO grant_record SELECT id,evidence FROM marker WHERE id=1",
                    [],
                )
                .unwrap();
            transaction
                .execute("DELETE FROM marker WHERE id=1", [])
                .unwrap();
            if mode == "after-commit" {
                transaction.commit().unwrap();
            }
            hold();
        }
        "read-marker" | "read-grant" => {
            let store = state::Store::open(&root.join("state.sqlite")).unwrap();
            let granted = mode == "read-grant";
            assert_eq!(
                store.snapshot().unwrap(),
                if granted { (0, 1) } else { (1, 0) }
            );
            store.assert_evidence(granted);
            ready();
        }
        _ => panic!("unknown child operation"),
    }
}
