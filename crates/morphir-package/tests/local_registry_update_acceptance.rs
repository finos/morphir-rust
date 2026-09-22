use cucumber::{World, given, then, when};
#[path = "local_registry/update_driver.rs"]
#[allow(dead_code)]
mod driver;
use driver::TestDriver;
#[derive(Debug, Default, World)]
struct UpdateWorld {
    driver: Option<TestDriver>,
}
impl UpdateWorld {
    fn driver(&mut self) -> &mut TestDriver {
        self.driver.as_mut().unwrap()
    }
}
#[given("a provisioned current registry and historical four-Library lock")]
fn provisioned(world: &mut UpdateWorld) {
    world.driver = Some(TestDriver::provisioned(None));
}
#[given("a current registry with a yanked frozen sibling")]
fn yanked(world: &mut UpdateWorld) {
    world.driver = Some(TestDriver::provisioned(Some("yanked-frozen")));
}
#[given("a current registry whose exact target requires changing a frozen sibling")]
fn conflict(world: &mut UpdateWorld) {
    let mut driver = TestDriver::provisioned(Some("scope-conflict"));
    driver.exact("1.4.0");
    world.driver = Some(driver);
}
#[when("I update the eligibility dependency")]
async fn update(world: &mut UpdateWorld) {
    world.driver().update().await;
}
#[then("only the target and necessary child move in the independent full lock")]
fn golden(world: &mut UpdateWorld) {
    world.driver().assert_golden("update.lock.json");
}
#[then("the yanked sibling remains pinned in the independent full lock")]
fn yanked_golden(world: &mut UpdateWorld) {
    world.driver().assert_golden("yanked-frozen.lock.json");
}
#[then("all four updated Libraries can be restored")]
async fn restore(world: &mut UpdateWorld) {
    world.driver().restore().await;
}
#[then("the update refuses the scope conflict without changing the old lock")]
fn refused(world: &mut UpdateWorld) {
    world.driver().assert_refused("ScopeConflict");
}
#[then("another update refuses the unresolved operation")]
async fn restart(world: &mut UpdateWorld) {
    world.driver().update().await;
    world.driver().assert_refused("unresolved prior operation");
}
#[tokio::main]
async fn main() {
    UpdateWorld::cucumber()
        .with_default_cli()
        .fail_on_skipped()
        .run_and_exit(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/local_registry_update.feature"
        ))
        .await;
}
