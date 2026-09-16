//! The IR YAML profile: one reader producing the JSON value tree, one canonical writer.
//!
//! See `docs/spec/ir/schemas/v4/yaml-profile.md`.

mod read;
mod scalar;
mod write;

pub use read::{MAX_DEPTH, read};
pub use write::write_canonical;

use crate::ir::v4::IRFile;
use crate::ir::{DiagnosticError, Warning};

/// Reads a profile-conforming YAML document as an [`IRFile`], with the decoder's warnings.
pub fn read_ir_file(text: &str) -> Result<(IRFile, Vec<Warning>), DiagnosticError> {
    let value = read(text).map_err(DiagnosticError)?;
    crate::ir::v4::decode_ir_file_with_warnings(&value)
}

/// Writes an [`IRFile`] as canonical profile YAML.
pub fn write_ir_file(file: &IRFile) -> String {
    let value = serde_json::to_value(file).expect("an IRFile serialises");
    write_canonical(&value)
}
