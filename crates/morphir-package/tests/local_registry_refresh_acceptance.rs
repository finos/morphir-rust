use cucumber::{World, given, then, when};
#[path = "local_registry/refresh_driver.rs"]
mod driver;
#[path = "local_registry/resolve_mothers.rs"]
#[allow(dead_code)]
mod mothers;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;
use driver::TestDriver;

#[derive(Debug, Default, World)]
struct RefreshWorld {
    driver: Option<TestDriver>,
}
impl RefreshWorld {
    fn driver(&mut self) -> &mut TestDriver {
        self.driver.as_mut().expect("provision first")
    }
}
#[given("a provisioned signed registry")]
fn provisioned(world: &mut RefreshWorld) {
    world.driver = Some(TestDriver::provisioned(false));
}
#[given("a provisioned registry with an authenticated revoked declaration")]
fn revoked(world: &mut RefreshWorld) {
    world.driver = Some(TestDriver::provisioned(true));
}
#[given("all package material is missing")]
fn missing(world: &mut RefreshWorld) {
    world.driver().remove_packages();
}
#[when("I explicitly refresh metadata")]
async fn refresh(world: &mut RefreshWorld) {
    world.driver().refresh().await;
}
#[when("the current child metadata is corrupted")]
fn corrupt(world: &mut RefreshWorld) {
    world.driver().corrupt_child();
}
#[then("only metadata observations are returned and the lock is unchanged")]
fn report(world: &mut RefreshWorld) {
    world.driver().assert_observation_only();
}
#[then("the exact metadata digests repeat")]
fn repeat(world: &mut RefreshWorld) {
    world.driver().assert_same_digests();
}
#[then("refresh refuses and retains its operation marker")]
fn refused(world: &mut RefreshWorld) {
    world.driver().assert_refused();
}
#[then("refresh refuses the unsupported revocation transition")]
fn revocation(world: &mut RefreshWorld) {
    world.driver().assert_revocation();
}
#[then("a restarted refresh refuses the unresolved operation")]
async fn restart(world: &mut RefreshWorld) {
    world.driver().assert_restart_refused().await;
}
#[tokio::main]
async fn main() {
    RefreshWorld::cucumber()
        // CI forwards libtest flags; always run the complete fixed feature.
        .with_default_cli()
        .fail_on_skipped()
        .run_and_exit(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/local_registry_refresh.feature"
        ))
        .await;
}
