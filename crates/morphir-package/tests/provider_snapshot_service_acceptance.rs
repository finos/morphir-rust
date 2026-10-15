use cucumber::{World, given, then, when};
#[path = "local_registry/snapshot_service_driver.rs"]
mod driver;
#[path = "local_registry/snapshot_mothers.rs"]
#[allow(dead_code)]
mod mothers;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;
#[derive(Debug, Default, World)]
struct ServiceWorld {
    driver: Option<driver::TestDriver>,
}
impl ServiceWorld {
    fn driver(&mut self) -> &mut driver::TestDriver {
        self.driver.as_mut().unwrap()
    }
}
#[given("a signed naming service fixture with provisioned trust")]
async fn provision(world: &mut ServiceWorld) {
    world.driver = Some(driver::TestDriver::provisioned().await);
}
#[when("I probe the executable without restore inputs")]
fn probe(world: &mut ServiceWorld) {
    world.driver().probe();
}
#[then("the executable returns capabilities without filesystem effects")]
fn capabilities(world: &mut ServiceWorld) {
    world.driver().assert_capabilities();
}
#[when("I request the exact locked naming release")]
fn restore(world: &mut ServiceWorld) {
    world.driver().restore("example.com/morphir/naming@1.0.0");
}
#[then("one binary response retains the original naming inventory")]
fn snapshot(world: &mut ServiceWorld) {
    world.driver().assert_snapshot();
}
#[when("I request a different release")]
fn other(world: &mut ServiceWorld) {
    world.driver().restore("example.com/morphir/other@1.0.0");
}
#[then("the executable refuses the release without output or publication")]
fn mismatch(world: &mut ServiceWorld) {
    world
        .driver()
        .assert_refused("metadata.restore_release_mismatch");
}
#[when("the current timestamp becomes invalid")]
fn corrupt(world: &mut ServiceWorld) {
    world.driver().corrupt();
}
#[then("the executable refuses fresh authentication without output or publication")]
fn refused(world: &mut ServiceWorld) {
    world
        .driver()
        .assert_refused("metadata.restore_authentication_failed");
}
#[tokio::main]
async fn main() {
    ServiceWorld::cucumber()
        .with_default_cli()
        .fail_on_skipped()
        .run_and_exit(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/provider_snapshot_service.feature"
        ))
        .await;
}
