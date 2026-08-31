#[path = "drivers/cache_ownership_registry_driver.rs"]
mod cache_ownership_registry_driver;

use cache_ownership_registry_driver::CacheOwnershipRegistryDriver;
use cucumber::{World, given, then, when};

#[derive(Debug, Default, World)]
struct CacheOwnershipRegistryWorld {
    driver: CacheOwnershipRegistryDriver,
}

#[given("registered and unknown files in a Morphir cache namespace")]
async fn given_registered_and_unknown_files(world: &mut CacheOwnershipRegistryWorld) {
    world.driver.given_registered_and_unknown_files();
}

#[given("a cache file whose owner released its registration")]
async fn given_released_cache_file(world: &mut CacheOwnershipRegistryWorld) {
    world.driver.given_released_cache_file();
}

#[when("I run cleanup through a guarded ownership session")]
async fn run_guarded_cleanup(world: &mut CacheOwnershipRegistryWorld) {
    world.driver.when_running_guarded_cleanup();
}

#[then("the registered cache file is removed")]
async fn registered_file_removed(world: &mut CacheOwnershipRegistryWorld) {
    world.driver.assert_registered_file_removed();
}

#[then("the unknown cache file remains")]
async fn unknown_file_remains(world: &mut CacheOwnershipRegistryWorld) {
    world.driver.assert_unknown_file_remains();
}

#[then("the released cache file remains unclassified")]
async fn released_file_remains_unclassified(world: &mut CacheOwnershipRegistryWorld) {
    world.driver.assert_released_file_remains_unclassified();
}

#[tokio::main]
async fn main() {
    CacheOwnershipRegistryWorld::run("tests/features/cache_ownership_registry.feature").await;
}
