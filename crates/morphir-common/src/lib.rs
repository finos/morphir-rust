pub mod config;
pub mod loader;
pub mod pipeline;
pub mod remote;
pub mod vfs;

pub type Result<T> = anyhow::Result<T>;
