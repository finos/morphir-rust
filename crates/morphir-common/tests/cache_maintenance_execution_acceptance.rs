#[path = "drivers/cache_execution_driver.rs"]
mod cache_execution_driver;

use cache_execution_driver::CacheExecutionDriver;
use cucumber::{World, given, then, when};

#[derive(Debug, Default, World)]
struct CacheExecutionWorld {
    driver: CacheExecutionDriver,
}

#[given("an owned cleanup candidate and an unknown cache entry")]
async fn given_owned_and_unknown(world: &mut CacheExecutionWorld) {
    world.driver.given_owned_and_unknown();
}

#[given("a cleanup candidate that acquires a lease after planning")]
async fn given_late_lease(world: &mut CacheExecutionWorld) {
    world.driver.given_late_lease();
}

#[given("two planner-selected cleanup candidates and a one-removal budget")]
async fn given_bounded_candidates(world: &mut CacheExecutionWorld) {
    world.driver.given_bounded_candidates();
}

#[given("content left by an interrupted cleanup trash run")]
async fn given_interrupted_trash(world: &mut CacheExecutionWorld) {
    world.driver.given_interrupted_trash();
}

#[when("I execute the cleanup plan")]
async fn execute_plan(world: &mut CacheExecutionWorld) {
    world.driver.when_executing();
}

#[then("the owned candidate is removed")]
async fn owned_candidate_is_removed(world: &mut CacheExecutionWorld) {
    world.driver.assert_owned_removed();
}

#[then("the unknown cache entry remains")]
async fn unknown_entry_remains(world: &mut CacheExecutionWorld) {
    world.driver.assert_unknown_remains();
}

#[then("the late lease defers removal")]
async fn late_lease_defers_removal(world: &mut CacheExecutionWorld) {
    world.driver.assert_late_lease_deferred();
}

#[then("the leased cache entry remains")]
async fn leased_entry_remains(world: &mut CacheExecutionWorld) {
    world.driver.assert_leased_remains();
}

#[then("one candidate is removed and one is deferred")]
async fn one_candidate_is_deferred(world: &mut CacheExecutionWorld) {
    world.driver.assert_one_removed_one_deferred();
}

#[then("the interrupted trash run is removed")]
async fn interrupted_trash_is_removed(world: &mut CacheExecutionWorld) {
    world.driver.assert_interrupted_trash_removed();
}

#[tokio::main]
async fn main() {
    CacheExecutionWorld::run("tests/features/cache_maintenance_execution.feature").await;
}
