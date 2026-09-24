//! Reads a `.feature.md` file (Markdown with Gherkin). Implemented in the next task.

use std::path::Path;

use crate::error::ReadError;
use crate::model::Document;
use crate::span::SourceText;

pub fn read(path: &Path, _source: &SourceText) -> Result<Document, ReadError> {
    Err(ReadError::UnknownFormat {
        path: path.to_owned(),
    })
}
