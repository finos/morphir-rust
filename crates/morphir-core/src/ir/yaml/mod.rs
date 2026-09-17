//! The IR YAML profile: one reader producing the JSON value tree, one canonical writer.
//!
//! See `docs/spec/ir/schemas/v4/yaml-profile.md`.

mod read;
mod scalar;
mod write;

pub use read::{MAX_DEPTH, read};
pub use write::write_canonical;

use crate::ir::v4::{IRFile, TypeEncoding, with_type_encoding};
use crate::ir::{DiagnosticError, Warning};

/// Reads a profile-conforming YAML document as an [`IRFile`], with the decoder's warnings.
pub fn read_ir_file(text: &str) -> Result<(IRFile, Vec<Warning>), DiagnosticError> {
    let value = read(text).map_err(DiagnosticError)?;
    crate::ir::v4::decode_ir_file_with_warnings(&value)
}

/// Writes an [`IRFile`] as canonical profile YAML.
///
/// The value tree is built under [`TypeEncoding::Compact`], which is the canonical spelling of a
/// type expression (decision 0005): a reference with no arguments and no attributes is
/// `morphir/SDK:basics#int`, not an expanded wrapper. The thread-local defaults to `Expanded`, so
/// a canonical writer has to say so.
pub fn write_ir_file(file: &IRFile) -> String {
    let value = with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(file))
        .expect("an IRFile serialises");
    write_canonical(&value)
}
