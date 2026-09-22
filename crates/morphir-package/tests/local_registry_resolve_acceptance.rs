use cucumber::{World, given, then, when};

#[path = "local_registry/resolve_driver.rs"]
mod driver;
#[path = "local_registry/resolve_mothers.rs"]
mod mothers;
#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod tuf_mothers;
use driver::TestDriver;

#[derive(Debug, Default, World)]
struct ResolveWorld {
    driver: Option<TestDriver>,
}
impl ResolveWorld {
    fn driver(&mut self) -> &mut TestDriver {
        self.driver.as_mut().expect("provision the registry first")
    }
}

#[given("a provisioned signed two-Library registry")]
fn provisioned(world: &mut ResolveWorld) {
    world.driver = Some(TestDriver::provisioned_registry());
}

#[given("the lock destination already contains a file")]
fn occupied(world: &mut ResolveWorld) {
    world.driver().occupy_destination();
}

#[given("the provider content is corrupt")]
fn corrupt(world: &mut ResolveWorld) {
    world.driver().corrupt_provider();
}

#[when("I resolve the published Library root")]
async fn resolve(world: &mut ResolveWorld) {
    world.driver().resolve().await;
}

#[when("I resolve the same root to another lock file")]
async fn repeat(world: &mut ResolveWorld) {
    world.driver().resolve_again().await;
}

#[then("the complete draft.3 lock is published")]
fn complete(world: &mut ResolveWorld) {
    world.driver().assert_complete_lock();
}

#[when("I restore the generated lock")]
async fn restore(world: &mut ResolveWorld) {
    world.driver().restore_generated_lock().await;
}

#[then("both Libraries are materialized")]
fn materialized(world: &mut ResolveWorld) {
    world.driver().assert_materialized();
}

#[then("the generated locks are byte-identical")]
fn deterministic(world: &mut ResolveWorld) {
    world.driver().assert_deterministic();
}

#[then("the existing lock bytes are unchanged")]
fn preserved(world: &mut ResolveWorld) {
    world.driver().assert_occupied_unchanged();
}

#[then("no trust operation was started")]
fn no_operation(world: &mut ResolveWorld) {
    world.driver().assert_no_operation();
}

#[then("no lock is published")]
fn no_lock(world: &mut ResolveWorld) {
    world.driver().assert_no_lock();
}

#[then("another resolution attempt refuses the unresolved operation")]
async fn refused(world: &mut ResolveWorld) {
    world.driver().assert_restart_refused().await;
}

#[tokio::main]
async fn main() {
    ResolveWorld::cucumber()
        // CI forwards libtest flags to every test binary. This dedicated
        // acceptance target always executes its complete fixed feature.
        .with_default_cli()
        .fail_on_skipped()
        .run_and_exit(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/local_registry_resolve.feature"
        ))
        .await;
}
