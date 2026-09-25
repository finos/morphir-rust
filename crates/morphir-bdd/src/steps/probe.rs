//! A probe step that proves a step library links into a test binary of another crate.

use cucumber::given;

use crate::world::MorphirWorld;

/// The component `Given the step library is linked` inserts. A test binary in another crate reads
/// it back to prove this crate's steps were kept by the linker.
#[derive(Debug, PartialEq, Eq)]
pub struct Linked(pub bool);

/// `Given the step library is linked` inserts [`Linked`]`(true)` into the scenario's context.
#[given("the step library is linked")]
fn step_library_is_linked(world: &mut MorphirWorld) {
    world.context.insert(Linked(true));
}
