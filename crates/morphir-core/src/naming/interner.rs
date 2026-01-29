use lasso::{ThreadedRodeo, Spur};
use std::sync::OnceLock;

static INTERNER: OnceLock<ThreadedRodeo> = OnceLock::new();

/// Returns a reference to the global string interner.
pub fn interner() -> &'static ThreadedRodeo {
    INTERNER.get_or_init(ThreadedRodeo::new)
}

/// A handle to an interned string (word).
pub type Word = Spur;

/// Interns a string and returns its handle.
pub fn intern(s: &str) -> Word {
    interner().get_or_intern(s)
}

/// Resolves an interned handle back to its string value.
pub fn resolve(word: Word) -> &'static str {
    interner().resolve(&word)
}
