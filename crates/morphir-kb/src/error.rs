//! Crate-level error type.
//!
//! Operational failures surface as `Error::Msg` with the exact user-facing message the
//! CLI prints (`error: <msg>`); structured variants exist only where a source error is
//! worth preserving. Builder modules must not extend this enum — use `Error::msg`.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Okf(#[from] morphir_okf::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Msg(String),
}

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Error::Msg(message.into())
    }
}
