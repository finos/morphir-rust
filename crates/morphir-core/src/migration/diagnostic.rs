use serde::{Deserialize, Serialize};

use crate::traversal::IrCursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum V4Encoding {
    #[default]
    Compact,
    Expanded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MigrationOptions {
    pub allow_partial: bool,
    pub encoding: V4Encoding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDiagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub path: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(skip)]
    pub recoverable: bool,
}

impl MigrationDiagnostic {
    pub fn error(code: &'static str, cursor: IrCursor, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            path: cursor.to_string(),
            message: message.into(),
            help: None,
            recoverable: false,
        }
    }

    pub fn recoverable(code: &'static str, cursor: IrCursor, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            path: cursor.to_string(),
            message: message.into(),
            help: None,
            recoverable: true,
        }
    }

    pub fn warning(code: &'static str, cursor: IrCursor, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            path: cursor.to_string(),
            message: message.into(),
            help: None,
            recoverable: true,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct MigrationReport {
    options: MigrationOptions,
    diagnostics: Vec<MigrationDiagnostic>,
}

impl MigrationReport {
    pub fn new(options: MigrationOptions) -> Self {
        Self {
            options,
            diagnostics: Vec::new(),
        }
    }

    pub fn push(&mut self, diagnostic: MigrationDiagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn diagnostics(&self) -> &[MigrationDiagnostic] {
        &self.diagnostics
    }

    pub fn can_publish(&self) -> bool {
        self.diagnostics.iter().all(|diagnostic| {
            diagnostic.severity != Severity::Error
                || (self.options.allow_partial && diagnostic.recoverable)
        })
    }
}
