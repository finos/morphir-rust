#[path = "drivers/cache_maintenance_state_driver.rs"]
mod cache_maintenance_state_driver;

use cache_maintenance_state_driver::CacheMaintenanceStateDriver;
use cucumber::{World, given, then, when};

#[derive(Debug, Default, World)]
struct CacheMaintenanceStateWorld {
    driver: CacheMaintenanceStateDriver,
}

#[given("a registered stale cache entry awaiting automatic maintenance")]
async fn given_registered_stale_entry(world: &mut CacheMaintenanceStateWorld) {
    world.driver.given_registered_stale_entry();
}

#[when("I run one automatic maintenance transaction")]
async fn run_automatic_transaction(world: &mut CacheMaintenanceStateWorld) {
    world.driver.when_running_transaction();
}

#[then("the registered cache entry is removed")]
async fn registered_entry_is_removed(world: &mut CacheMaintenanceStateWorld) {
    world.driver.assert_registered_entry_removed();
}

#[then("the successful automatic run timestamp is durable")]
async fn successful_timestamp_is_durable(world: &mut CacheMaintenanceStateWorld) {
    world.driver.assert_success_timestamp_durable();
}

#[tokio::main]
async fn main() {
    CacheMaintenanceStateWorld::run("tests/features/cache_maintenance_state.feature").await;
}
