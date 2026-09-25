//! Runs the BDD suites for `morphir-bdd` itself. It proves the spike question: a step library
//! defined in this crate links into this integration test binary.

use cucumber::writer::Stats as _;
use cucumber::{World as _, then};
use morphir_bdd::steps::probe::Linked;
use morphir_bdd::world::MorphirWorld;

#[then("the linked flag is set")]
fn linked_flag(world: &mut MorphirWorld) {
    assert_eq!(world.context.get::<Linked>(), Some(&Linked(true)));
}

#[tokio::main]
async fn main() {
    morphir_bdd::link();
    let writer = MorphirWorld::cucumber()
        .fail_on_skipped()
        .run(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/features/link.feature"
        ))
        .await;
    // The second scenario must fail: an undefined step is not a pass or a silent skip.
    assert_eq!(
        writer.passed_steps(),
        2,
        "the library step and the local step ran"
    );
    assert!(
        writer.execution_has_failed(),
        "a missing step must fail its scenario"
    );
}
