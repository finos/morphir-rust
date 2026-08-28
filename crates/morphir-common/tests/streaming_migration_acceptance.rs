mod drivers;

use cucumber::{World, given, then, when};
use drivers::migration_driver::MigrationDriver;

#[derive(Debug, Default, World)]
struct MigrationWorld {
    driver: MigrationDriver,
}

#[given("a concrete Classic v3 JSON distribution")]
async fn given_classic_v3_json(world: &mut MigrationWorld) {
    world.driver.given_classic_v3_json();
}

#[when("I stream it through the v3 to v4 pipeline into native YAML")]
async fn stream_to_native_v4_yaml(world: &mut MigrationWorld) {
    world.driver.when_streaming_to_native_v4_yaml();
}

#[then("the migration output is concrete v4 YAML")]
async fn output_is_concrete_v4_yaml(world: &mut MigrationWorld) {
    world.driver.assert_concrete_v4_yaml();
}

#[then("the migration pipeline retains at most one module")]
async fn pipeline_is_module_bounded(world: &mut MigrationWorld) {
    world.driver.assert_module_bounded();
}

#[then("the migration report permits publication")]
async fn report_permits_publication(world: &mut MigrationWorld) {
    world.driver.assert_report_publishable();
}

#[tokio::main]
async fn main() {
    MigrationWorld::run("tests/features/streaming_migration.feature").await;
}
