#[path = "drivers/cache_inventory_driver.rs"]
mod cache_inventory_driver;

use cache_inventory_driver::CacheInventoryDriver;
use cucumber::{World, given, then, when};

#[derive(Debug, Default, World)]
struct CacheInventoryWorld {
    driver: CacheInventoryDriver,
}

#[given("a cache namespace with disposable, leased, and unknown entries")]
async fn given_classified_entries(world: &mut CacheInventoryWorld) {
    world.driver.given_classified_entries();
}

#[given("a cache namespace that exceeds a one-entry inventory budget")]
async fn given_entry_budget_overflow(world: &mut CacheInventoryWorld) {
    world.driver.given_entry_budget_overflow();
}

#[given("a cache namespace root that links outside Morphir Home")]
async fn given_link_like_namespace_root(world: &mut CacheInventoryWorld) {
    world.driver.given_link_like_namespace_root();
}

#[when("I inventory the cache namespace")]
async fn inventory_namespace(world: &mut CacheInventoryWorld) {
    world.driver.when_inventorying();
}

#[when("I inventory the cache namespace with a one-entry budget")]
async fn inventory_namespace_with_one_entry(world: &mut CacheInventoryWorld) {
    world.driver.when_inventorying_with_one_entry();
}

#[then("the disposable entry is measured as removable ownership")]
async fn disposable_entry_is_measured(world: &mut CacheInventoryWorld) {
    world.driver.assert_disposable_entry();
}

#[then("the leased entry remains protected")]
async fn leased_entry_is_protected(world: &mut CacheInventoryWorld) {
    world.driver.assert_leased_entry();
}

#[then("the unknown entry remains unclassified")]
async fn unknown_entry_is_unclassified(world: &mut CacheInventoryWorld) {
    world.driver.assert_unknown_entry();
}

#[then("inventory fails closed with an entry-limit diagnostic")]
async fn inventory_fails_at_entry_limit(world: &mut CacheInventoryWorld) {
    world.driver.assert_entry_limit_failure();
}

#[then("inventory refuses the link-like namespace root")]
async fn inventory_refuses_link(world: &mut CacheInventoryWorld) {
    world.driver.assert_link_refused();
}

#[then("content outside Morphir Home remains unchanged")]
async fn outside_content_is_unchanged(world: &mut CacheInventoryWorld) {
    world.driver.assert_outside_unchanged();
}

#[tokio::main]
async fn main() {
    CacheInventoryWorld::run("tests/features/cache_maintenance_inventory.feature").await;
}
