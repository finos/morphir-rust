use cucumber::{World, given, then, when};
#[path = "local_registry/provider_fixture_driver.rs"]
mod driver;
#[path = "../examples/provider-snapshot-fixture.rs"]
#[allow(dead_code)]
mod fixture;

#[derive(Debug, Default, World)]
struct FixtureWorld {
    driver: Option<driver::TestDriver>,
}
impl FixtureWorld {
    fn driver(&mut self) -> &mut driver::TestDriver {
        self.driver.as_mut().unwrap()
    }
}
#[given("a new signed provider fixture destination")]
fn new_destination(world: &mut FixtureWorld) {
    world.driver = Some(driver::TestDriver::new());
}
#[given("an explicitly provisioned signed provider fixture")]
async fn provisioned(world: &mut FixtureWorld) {
    new_destination(world);
    world.driver().export(Some("--provision-test-trust")).await;
    world.driver().remember_existing();
}
#[when("I export test artifacts without a trust flag")]
async fn artifacts(world: &mut FixtureWorld) {
    world.driver().export(None).await;
}
#[when("I export with explicit test trust provisioning")]
async fn explicit(world: &mut FixtureWorld) {
    world.driver().export(Some("--provision-test-trust")).await;
}
#[then("signed interoperability artifacts exist without a trust database")]
fn artifacts_only(world: &mut FixtureWorld) {
    world.driver().assert_artifacts_only();
}
#[then("the shared service freshly authenticates the exported naming release")]
fn authenticate(world: &mut FixtureWorld) {
    world.driver().assert_fresh_authentication();
}
#[when("I export again to the same destination")]
async fn repeated(world: &mut FixtureWorld) {
    world.driver().export(Some("--provision-test-trust")).await;
}
#[then("the export refuses without changing any existing artifact or trust state")]
fn unchanged(world: &mut FixtureWorld) {
    world.driver().assert_preserved();
}
#[when("I request an unknown trust option")]
async fn unknown(world: &mut FixtureWorld) {
    world.driver().export(Some("--implicit-trust")).await;
}
#[then("the export refuses without creating the destination")]
fn no_destination(world: &mut FixtureWorld) {
    world.driver().assert_no_destination();
}
#[tokio::main]
async fn main() {
    FixtureWorld::cucumber()
        .with_default_cli()
        .fail_on_skipped()
        .run_and_exit(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/provider_fixture.feature"
        ))
        .await;
}
