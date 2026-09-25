//! Morphir's base step libraries. A test binary calls [`link`] so the linker keeps these steps.

pub mod cli;
pub mod files;
pub mod output;
pub mod probe;

/// Keeps this crate's step statics in a test binary. Call it once from `main`.
#[inline(never)]
pub fn link() {
    std::hint::black_box(probe::Linked(true));
    std::hint::black_box(
        files::workspace as fn(&mut crate::world::MorphirWorld) -> &files::Workspace,
    );
    std::hint::black_box(output::LastOutput::default);
    std::hint::black_box(cli::split_command_line as fn(&str) -> Result<Vec<String>, String>);
}
