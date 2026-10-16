//! Qualify packaged release executables. Not run by default `cargo test`: select it
//! with `--test provider_release_acceptance` and set `MORPHIR_PROVIDER_RELEASE_SERVICE`
//! and `MORPHIR_PROVIDER_RELEASE_FIXTURE` to the extracted executables. With neither
//! variable set it qualifies nothing; with only one set it fails.
use cucumber::{World, given, then, when};
#[path = "local_registry/release_driver.rs"]
mod driver;

#[derive(Debug, Default, World)]
struct ReleaseWorld {
    driver: Option<driver::TestDriver>,
}
impl ReleaseWorld {
    fn driver(&mut self) -> &mut driver::TestDriver {
        self.driver.as_mut().unwrap()
    }
}
#[given("the packaged service and test fixture executables")]
fn packaged(world: &mut ReleaseWorld) {
    world.driver = Some(driver::TestDriver::from_environment());
}
#[when("I probe the packaged service")]
fn probe(world: &mut ReleaseWorld) {
    world.driver().probe();
}
#[then("the packaged service reports the snapshot profile")]
fn capabilities(world: &mut ReleaseWorld) {
    world.driver().assert_capabilities();
}
#[when("I export packaged test artifacts without a trust flag")]
fn artifacts(world: &mut ReleaseWorld) {
    world.driver().export_artifacts_only();
}
#[then("the export contains signed artifacts without a trust database")]
fn artifacts_only(world: &mut ReleaseWorld) {
    world.driver().assert_artifacts_only();
}
#[when("I export packaged test artifacts with an unknown option")]
fn unknown(world: &mut ReleaseWorld) {
    world.driver().export_unknown_option();
}
#[then("the exporter fails without creating the destination")]
fn no_destination(world: &mut ReleaseWorld) {
    world.driver().assert_no_destination();
}
#[when(expr = "I export the packaged {string} variant with fresh test trust")]
fn export_variant(world: &mut ReleaseWorld, variant: String) {
    world.driver().export_variant(&variant);
}
#[when("the exported timestamp becomes invalid")]
fn invalidate(world: &mut ReleaseWorld) {
    world.driver().invalidate_timestamp();
}
#[when("I request the naming release from the packaged service")]
fn restore(world: &mut ReleaseWorld) {
    world.driver().restore();
}
#[then(expr = "the response retains the exported {string} bytes and checked core")]
fn retained(world: &mut ReleaseWorld, variant: String) {
    world.driver().assert_variant(&variant);
}
#[then("the packaged service refuses authentication without output")]
fn refused(world: &mut ReleaseWorld) {
    world
        .driver()
        .assert_refused("metadata.restore_authentication_failed");
}
#[tokio::main]
async fn main() {
    let selected = [driver::SERVICE_VARIABLE, driver::FIXTURE_VARIABLE]
        .map(|variable| std::env::var_os(variable).is_some());
    if selected == [false, false] {
        // `cargo test --test '*'` also selects this target. Without packaged
        // executables there is nothing to qualify; release workflows set both.
        eprintln!(
            "provider_release_acceptance: no packaged executables selected; set {} and {}",
            driver::SERVICE_VARIABLE,
            driver::FIXTURE_VARIABLE
        );
        return;
    }
    ReleaseWorld::cucumber()
        .with_default_cli()
        .fail_on_skipped()
        .run_and_exit(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/provider_release.feature"
        ))
        .await;
}
