//! Version migration implemented as a format-neutral semantic transform.

use std::sync::{Arc, OnceLock};

use morphir_core::ir::v4;
use morphir_core::migration::{
    MigrationContext, MigrationOptions, MigrationReport, migrate_access, migrate_module_definition,
    migrate_package_specification, migrate_path,
};
use morphir_core::naming::PackageName;
use morphir_core::traversal::{
    DependencyEvent, DistributionHeader, ModuleEvent, SemanticEvent, SemanticEventKind,
};

use super::{EventTransform, Retention, Stage, TransportDiagnostic};

/// Shared read-only access to a migration report after a transform finishes.
#[derive(Clone, Default)]
pub struct MigrationReportHandle(Arc<OnceLock<MigrationReport>>);

impl MigrationReportHandle {
    /// Return the completed report, or `None` while migration is still running.
    pub fn get(&self) -> Option<&MigrationReport> {
        self.0.get()
    }
}

/// Module-bounded transform from concrete Classic v3 events to concrete v4 events.
///
/// The transform composes between any event source and sink, so physical formats
/// stay outside the migration logic:
///
/// ```
/// use std::io::Cursor;
///
/// use morphir_common::ir_transport::{
///     ClassicToV4, CodecOptions, FormatId, IrCodec, IrVersion, JsonCodec, Layout,
///     Pipeline, YamlCodec,
/// };
/// use morphir_core::migration::MigrationOptions;
///
/// let input = br#"{
///   "formatVersion": 3,
///   "distribution": ["Library", [["example"]], [], {"modules": []}]
/// }"#;
/// let input_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
/// let output_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::yaml());
/// let transform = ClassicToV4::new(MigrationOptions::default());
/// let report = transform.report_handle();
/// let mut pipeline = Pipeline::new().with_transform(transform);
/// let mut output = Vec::new();
/// let mut encoder = YamlCodec::new()
///     .encoder(&mut output, &output_options)
///     .expect("create YAML sink");
///
/// {
///     let mut sink = pipeline.sink(encoder.as_mut()).expect("create migration sink");
///     JsonCodec::new()
///         .decode(&mut Cursor::new(input), &input_options, &mut sink)
///         .expect("stream v3 JSON through the migration pipeline");
/// }
/// drop(encoder);
///
/// assert!(report.get().expect("completed report").can_publish());
/// assert!(String::from_utf8(output).unwrap().starts_with("formatVersion: 4\n"));
/// ```
pub struct ClassicToV4 {
    context: MigrationContext,
    report: MigrationReportHandle,
}

impl ClassicToV4 {
    /// Create a streaming v3-to-v4 transform.
    pub fn new(options: MigrationOptions) -> Self {
        Self {
            context: MigrationContext::new(options),
            report: MigrationReportHandle::default(),
        }
    }

    /// Return a handle populated when the transform finishes.
    pub fn report_handle(&self) -> MigrationReportHandle {
        self.report.clone()
    }

    fn unexpected(event: &SemanticEvent) -> TransportDiagnostic {
        TransportDiagnostic::error(
            "morphir::ir::migration::expected_classic_v3",
            Stage::Migration,
            event.cursor().clone(),
            "the v3-to-v4 transform received a non-v3 or out-of-order event",
        )
        .with_guidance("select v3 as the input version or remove the v3-to-v4 transform")
    }
}

impl EventTransform for ClassicToV4 {
    fn retention(&self) -> Retention {
        Retention::Module
    }

    fn transform(
        &mut self,
        event: SemanticEvent,
        emit: &mut dyn FnMut(SemanticEvent) -> Result<(), TransportDiagnostic>,
    ) -> Result<(), TransportDiagnostic> {
        self.context.cursor = event.cursor().clone();
        let (cursor, kind) = event.into_parts();
        let migrated = match kind {
            SemanticEventKind::Begin(DistributionHeader::ClassicV3Library { package }) => {
                SemanticEventKind::Begin(DistributionHeader::V4Library {
                    format_version: v4::FormatVersion::Integer(4),
                    package: PackageName::new(migrate_path(&package)),
                })
            }
            SemanticEventKind::Dependency(DependencyEvent::ClassicV3 {
                package,
                specification,
            }) => SemanticEventKind::Dependency(DependencyEvent::V4 {
                package: migrate_path(&package).to_canonical_string(),
                specification: migrate_package_specification(&specification, &mut self.context)
                    .map_err(TransportDiagnostic::from)?,
            }),
            SemanticEventKind::Module(ModuleEvent::ClassicV3(module)) => {
                let path = migrate_path(&module.path).to_canonical_string();
                let migrated = v4::AccessControlled {
                    access: migrate_access(&module.definition.access),
                    value: migrate_module_definition(&module.definition.value, &mut self.context)
                        .map_err(TransportDiagnostic::from)?,
                };
                SemanticEventKind::Module(ModuleEvent::V4Definition {
                    path,
                    module: migrated,
                })
            }
            SemanticEventKind::End => SemanticEventKind::End,
            unexpected => {
                return Err(Self::unexpected(&SemanticEvent::new(cursor, unexpected)));
            }
        };
        emit(SemanticEvent::new(cursor, migrated))
    }

    fn finish(
        &mut self,
        _emit: &mut dyn FnMut(SemanticEvent) -> Result<(), TransportDiagnostic>,
    ) -> Result<(), TransportDiagnostic> {
        self.report.0.set(self.context.report.clone()).map_err(|_| {
            TransportDiagnostic::error(
                "morphir::ir::migration::already_finished",
                Stage::Migration,
                self.context.cursor.clone(),
                "the v3-to-v4 transform was finished more than once",
            )
            .with_guidance("create a new transform for each pipeline run")
        })
    }
}
