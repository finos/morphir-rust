//! Format-neutral events for traversing versioned Morphir IR.

use crate::ir::{classic, v4};
use crate::naming::PackageName;

use super::IrCursor;

/// A concrete Classic v3 module released as one streaming unit.
pub type ClassicV3Module = classic::ModuleEntry<classic::Attrs, classic::Type<classic::Attrs>>;

/// Distribution metadata emitted before dependencies and modules.
#[derive(Debug, Clone, PartialEq)]
pub enum DistributionHeader {
    /// A Classic v3 library distribution.
    ClassicV3Library { package: classic::Path },
    /// A v4 library distribution.
    V4Library {
        format_version: v4::FormatVersion,
        package: PackageName,
    },
    /// A v4 specification distribution.
    V4Specs {
        format_version: v4::FormatVersion,
        package: PackageName,
    },
    /// A v4 application distribution.
    V4Application {
        format_version: v4::FormatVersion,
        package: PackageName,
        entry_points: v4::EntryPoints,
    },
}

/// One dependency specification in a versioned distribution.
#[derive(Debug, Clone, PartialEq)]
pub enum DependencyEvent {
    /// A dependency represented with Classic v3 concrete types.
    ClassicV3 {
        package: classic::Path,
        specification: classic::PackageSpecification<classic::Attrs>,
    },
    /// A dependency represented with v4 concrete types.
    V4 {
        package: String,
        specification: v4::PackageSpecification,
    },
}

/// One module in a versioned distribution.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleEvent {
    /// A Classic v3 module definition.
    ClassicV3(ClassicV3Module),
    /// A v4 module definition and its access control.
    V4Definition {
        path: String,
        module: v4::AccessControlled<v4::ModuleDefinition>,
    },
    /// A v4 module specification.
    V4Specification {
        path: String,
        module: v4::ModuleSpecification,
    },
}

/// The semantic payload of one format-neutral traversal event.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticEventKind {
    /// Starts a distribution.
    Begin(DistributionHeader),
    /// Provides one dependency specification.
    Dependency(DependencyEvent),
    /// Provides one module.
    Module(ModuleEvent),
    /// Ends the distribution.
    End,
}

/// A semantic IR event paired with its version-independent location.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticEvent {
    cursor: IrCursor,
    kind: SemanticEventKind,
}

impl SemanticEvent {
    /// Create a semantic event at the supplied cursor.
    pub fn new(cursor: IrCursor, kind: SemanticEventKind) -> Self {
        Self { cursor, kind }
    }

    /// Return the semantic location of this event.
    pub fn cursor(&self) -> &IrCursor {
        &self.cursor
    }

    /// Return the event payload.
    pub fn kind(&self) -> &SemanticEventKind {
        &self.kind
    }

    /// Consume this value and return its cursor and payload.
    pub fn into_parts(self) -> (IrCursor, SemanticEventKind) {
        (self.cursor, self.kind)
    }
}
