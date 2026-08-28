//! Conversion between concrete versioned IR values and semantic transport events.

use indexmap::IndexMap;
use morphir_core::ir::{classic, v4};
use morphir_core::traversal::{
    CursorSegment, DependencyEvent, DistributionHeader, IrCursor, ModuleEvent, SemanticEvent,
    SemanticEventKind,
};

use super::{EventSink, EventSource, IrVersion, Stage, TransportDiagnostic};

pub(crate) enum SemanticFile {
    ClassicV3(classic::Distribution),
    V4(v4::IRFile),
}

fn event_error(
    code: &'static str,
    stage: Stage,
    cursor: IrCursor,
    message: impl Into<String>,
) -> TransportDiagnostic {
    TransportDiagnostic::error(code, stage, cursor, message)
        .with_guidance("verify the event order and selected concrete IR version")
}

pub(crate) fn emit_classic_v3(
    file: classic::Distribution,
    sink: &mut dyn EventSink,
) -> Result<(), TransportDiagnostic> {
    if file.format_version != 3 {
        return Err(event_error(
            "morphir::ir::codec::version_mismatch",
            Stage::Normalization,
            IrCursor::root(),
            format!(
                "the selected v3 codec received formatVersion {}",
                file.format_version
            ),
        ));
    }

    let distribution_cursor = IrCursor::root().child(CursorSegment::Distribution);
    match file.distribution {
        classic::DistributionBody::Library(package, dependencies, definition) => {
            sink.accept(SemanticEvent::new(
                distribution_cursor.clone(),
                SemanticEventKind::Begin(DistributionHeader::ClassicV3Library {
                    package: package.clone(),
                }),
            ))?;
            for (dependency, specification) in dependencies {
                let cursor = distribution_cursor
                    .clone()
                    .child(CursorSegment::Dependency(dependency.to_string()));
                sink.accept(SemanticEvent::new(
                    cursor,
                    SemanticEventKind::Dependency(DependencyEvent::ClassicV3 {
                        package: dependency,
                        specification,
                    }),
                ))?;
            }
            for module in definition.modules {
                let cursor = distribution_cursor
                    .clone()
                    .child(CursorSegment::Module(module.path.to_string()));
                sink.accept(SemanticEvent::new(
                    cursor,
                    SemanticEventKind::Module(ModuleEvent::ClassicV3(module)),
                ))?;
            }
        }
    }
    sink.accept(SemanticEvent::new(
        distribution_cursor,
        SemanticEventKind::End,
    ))?;
    sink.finish()
}

pub(crate) fn emit_v4(
    file: v4::IRFile,
    sink: &mut dyn EventSink,
) -> Result<(), TransportDiagnostic> {
    let distribution_cursor = IrCursor::root().child(CursorSegment::Distribution);
    let format_version = file.format_version;
    match file.distribution {
        v4::Distribution::Library(content) => {
            sink.accept(SemanticEvent::new(
                distribution_cursor.clone(),
                SemanticEventKind::Begin(DistributionHeader::V4Library {
                    format_version,
                    package: content.package_name,
                }),
            ))?;
            emit_v4_dependencies(content.dependencies, &distribution_cursor, sink)?;
            for (path, module) in content.def.modules {
                sink.accept(SemanticEvent::new(
                    distribution_cursor
                        .clone()
                        .child(CursorSegment::Module(path.clone())),
                    SemanticEventKind::Module(ModuleEvent::V4Definition { path, module }),
                ))?;
            }
        }
        v4::Distribution::Specs(content) => {
            sink.accept(SemanticEvent::new(
                distribution_cursor.clone(),
                SemanticEventKind::Begin(DistributionHeader::V4Specs {
                    format_version,
                    package: content.package_name,
                }),
            ))?;
            emit_v4_dependencies(content.dependencies, &distribution_cursor, sink)?;
            for (path, module) in content.spec.modules {
                sink.accept(SemanticEvent::new(
                    distribution_cursor
                        .clone()
                        .child(CursorSegment::Module(path.clone())),
                    SemanticEventKind::Module(ModuleEvent::V4Specification { path, module }),
                ))?;
            }
        }
        v4::Distribution::Application(content) => {
            sink.accept(SemanticEvent::new(
                distribution_cursor.clone(),
                SemanticEventKind::Begin(DistributionHeader::V4Application {
                    format_version,
                    package: content.package_name,
                    entry_points: content.entry_points,
                }),
            ))?;
            emit_v4_dependencies(content.dependencies, &distribution_cursor, sink)?;
            for (path, module) in content.def.modules {
                sink.accept(SemanticEvent::new(
                    distribution_cursor
                        .clone()
                        .child(CursorSegment::Module(path.clone())),
                    SemanticEventKind::Module(ModuleEvent::V4Definition { path, module }),
                ))?;
            }
        }
    }
    sink.accept(SemanticEvent::new(
        distribution_cursor,
        SemanticEventKind::End,
    ))?;
    sink.finish()
}

fn emit_v4_dependencies(
    dependencies: v4::Dependencies,
    parent: &IrCursor,
    sink: &mut dyn EventSink,
) -> Result<(), TransportDiagnostic> {
    for (package, specification) in dependencies {
        sink.accept(SemanticEvent::new(
            parent
                .clone()
                .child(CursorSegment::Dependency(package.clone())),
            SemanticEventKind::Dependency(DependencyEvent::V4 {
                package,
                specification,
            }),
        ))?;
    }
    Ok(())
}

pub(crate) fn collect(
    source: &mut dyn EventSource,
    expected_version: IrVersion,
) -> Result<SemanticFile, TransportDiagnostic> {
    let first = source.next_event()?.ok_or_else(|| {
        event_error(
            "morphir::ir::codec::missing_begin",
            Stage::Encoding,
            IrCursor::root(),
            "the semantic event source was empty",
        )
    })?;
    let (cursor, kind) = first.into_parts();
    let SemanticEventKind::Begin(header) = kind else {
        return Err(event_error(
            "morphir::ir::codec::missing_begin",
            Stage::Encoding,
            cursor,
            "the first semantic event was not a distribution header",
        ));
    };

    match (expected_version, header) {
        (IrVersion::V3, DistributionHeader::ClassicV3Library { package }) => {
            collect_classic_v3(source, package)
        }
        (IrVersion::V4, header @ DistributionHeader::V4Library { .. })
        | (IrVersion::V4, header @ DistributionHeader::V4Specs { .. })
        | (IrVersion::V4, header @ DistributionHeader::V4Application { .. }) => {
            collect_v4(source, header)
        }
        _ => Err(event_error(
            "morphir::ir::codec::version_mismatch",
            Stage::Encoding,
            cursor,
            "semantic events do not match the selected concrete IR version",
        )),
    }
}

fn collect_classic_v3(
    source: &mut dyn EventSource,
    package: classic::Path,
) -> Result<SemanticFile, TransportDiagnostic> {
    let mut dependencies = Vec::new();
    let mut modules = Vec::new();
    while let Some(event) = source.next_event()? {
        let (cursor, kind) = event.into_parts();
        match kind {
            SemanticEventKind::Dependency(DependencyEvent::ClassicV3 {
                package,
                specification,
            }) => dependencies.push((package, specification)),
            SemanticEventKind::Module(ModuleEvent::ClassicV3(module)) => modules.push(module),
            SemanticEventKind::End => {
                ensure_finished(source, cursor)?;
                return Ok(SemanticFile::ClassicV3(classic::Distribution {
                    format_version: 3,
                    distribution: classic::DistributionBody::Library(
                        package,
                        dependencies,
                        classic::PackageDefinition { modules },
                    ),
                }));
            }
            _ => {
                return Err(event_error(
                    "morphir::ir::codec::invalid_event",
                    Stage::Encoding,
                    cursor,
                    "v3 event stream contained a v4 or out-of-order event",
                ));
            }
        }
    }
    Err(missing_end())
}

fn collect_v4(
    source: &mut dyn EventSource,
    header: DistributionHeader,
) -> Result<SemanticFile, TransportDiagnostic> {
    let mut dependencies = IndexMap::new();
    let mut definitions = IndexMap::new();
    let mut specifications = IndexMap::new();
    while let Some(event) = source.next_event()? {
        let (cursor, kind) = event.into_parts();
        match kind {
            SemanticEventKind::Dependency(DependencyEvent::V4 {
                package,
                specification,
            }) => {
                dependencies.insert(package, specification);
            }
            SemanticEventKind::Module(ModuleEvent::V4Definition { path, module }) => {
                definitions.insert(path, module);
            }
            SemanticEventKind::Module(ModuleEvent::V4Specification { path, module }) => {
                specifications.insert(path, module);
            }
            SemanticEventKind::End => {
                ensure_finished(source, cursor)?;
                let (format_version, distribution) = match header {
                    DistributionHeader::V4Library {
                        format_version,
                        package,
                    } if specifications.is_empty() => (
                        format_version,
                        v4::Distribution::Library(v4::LibraryContent {
                            package_name: package,
                            dependencies,
                            def: v4::PackageDefinition {
                                modules: definitions,
                            },
                        }),
                    ),
                    DistributionHeader::V4Specs {
                        format_version,
                        package,
                    } if definitions.is_empty() => (
                        format_version,
                        v4::Distribution::Specs(v4::SpecsContent {
                            package_name: package,
                            dependencies,
                            spec: v4::PackageSpecification {
                                modules: specifications,
                            },
                        }),
                    ),
                    DistributionHeader::V4Application {
                        format_version,
                        package,
                        entry_points,
                    } if specifications.is_empty() => (
                        format_version,
                        v4::Distribution::Application(v4::ApplicationContent {
                            package_name: package,
                            dependencies,
                            def: v4::PackageDefinition {
                                modules: definitions,
                            },
                            entry_points,
                        }),
                    ),
                    _ => {
                        return Err(event_error(
                            "morphir::ir::codec::invalid_event",
                            Stage::Encoding,
                            IrCursor::root(),
                            "v4 module events do not match the distribution kind",
                        ));
                    }
                };
                return Ok(SemanticFile::V4(v4::IRFile {
                    format_version,
                    distribution,
                }));
            }
            _ => {
                return Err(event_error(
                    "morphir::ir::codec::invalid_event",
                    Stage::Encoding,
                    cursor,
                    "v4 event stream contained a v3 or out-of-order event",
                ));
            }
        }
    }
    Err(missing_end())
}

fn ensure_finished(
    source: &mut dyn EventSource,
    cursor: IrCursor,
) -> Result<(), TransportDiagnostic> {
    if source.next_event()?.is_some() {
        return Err(event_error(
            "morphir::ir::codec::trailing_event",
            Stage::Encoding,
            cursor,
            "semantic events appeared after the distribution end",
        ));
    }
    Ok(())
}

fn missing_end() -> TransportDiagnostic {
    event_error(
        "morphir::ir::codec::missing_end",
        Stage::Encoding,
        IrCursor::root(),
        "the semantic event source ended before the distribution end event",
    )
}
