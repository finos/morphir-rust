//! A probe step that proves a step library links into a test binary of another crate.

use cucumber::given;

use crate::world::MorphirWorld;

#[derive(Debug, PartialEq, Eq)]
pub struct Linked(pub bool);

#[given("the step library is linked")]
fn step_library_is_linked(world: &mut MorphirWorld) {
    world.context.insert(Linked(true));
}
