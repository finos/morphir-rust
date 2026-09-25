//! Morphir's base step libraries. A test binary calls [`link`] so the linker keeps these steps.

pub mod probe;

/// Keeps this crate's step statics in a test binary. Call it once from `main`.
#[inline(never)]
pub fn link() {
    std::hint::black_box(probe::Linked(true));
}
