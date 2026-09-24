//! Conversion between concrete versioned IR values and semantic transport events.

use indexmap::IndexMap;
use morphir_core::ir::{classic, v4};
use morphir_core::traversal::{
    CursorSegment, DependencyEvent, DistributionHeader, IrCursor, ModuleEvent, SemanticEvent,
    SemanticEventKind,
};

use super::ClassicV3ModuleVisitor;
use super::{EventSink, EventSource, IrVersion, Stage, TransportDiagnostic};

pub(crate) enum SemanticFile {
    ClassicV3(classic::Distribution),
    V4(v4::IRFile),
}

pub(crate) struct ClassicEventVisitor<'sink> {
    sink: &'sink mut dyn EventSink,
    cursor: IrCursor,
    failure: Option<TransportDiagnostic>,
}

impl<'sink> ClassicEventVisitor<'sink> {
    pub(crate) fn new(sink: &'sink mut dyn EventSink) -> Self {
        Self {
            sink,
            cursor: IrCursor::root().child(CursorSegment::Distribution),
            failure: None,
        }
    }

    pub(crate) fn take_failure(&mut self) -> Option<TransportDiagnostic> {
        self.failure.take()
    }

    fn accept(&mut self, event: SemanticEvent) -> Result<(), String> {
        self.sink.accept(event).map_err(|diagnostic| {
            let message = diagnostic.message().to_owned();
            self.failure = Some(diagnostic);
            message
        })
    }

    fn begin_with(
        &mut self,
        header: DistributionHeader,
        dependencies: &[(classic::Path, classic::PackageSpecification<classic::Attrs>)],
    ) -> Result<(), String> {
        self.accept(SemanticEvent::new(
            self.cursor.clone(),
            SemanticEventKind::Begin(header),
        ))?;
        for (package, specification) in dependencies {
            self.accept(SemanticEvent::new(
                self.cursor
                    .clone()
                    .child(CursorSegment::Dependency(package.to_string())),
                SemanticEventKind::Dependency(DependencyEvent::ClassicV3 {
                    package: package.clone(),
                    specification: specification.clone(),
                }),
            ))?;
        }
        Ok(())
    }
}

impl ClassicV3ModuleVisitor for ClassicEventVisitor<'_> {
    type Output = Result<(), TransportDiagnostic>;

    fn begin(
        &mut self,
        package: &classic::Path,
        dependencies: &[(classic::Path, classic::PackageSpecification<classic::Attrs>)],
    ) -> Result<(), String> {
        self.begin_with(
            DistributionHeader::ClassicV3Library {
                package: package.clone(),
            },
            dependencies,
        )
    }

    fn begin_specs(
        &mut self,
        package: &classic::Path,
        dependencies: &[(classic::Path, classic::PackageSpecification<classic::Attrs>)],
    ) -> Result<(), String> {
        self.begin_with(
            DistributionHeader::ClassicV3Specs {
                package: package.clone(),
            },
            dependencies,
        )
    }

    fn visit_module(
        &mut self,
        module: classic::ModuleEntry<classic::Attrs, classic::Type<classic::Attrs>>,
    ) -> Result<(), String> {
        self.accept(SemanticEvent::new(
            self.cursor
                .clone()
                .child(CursorSegment::Module(module.path.to_string())),
            SemanticEventKind::Module(ModuleEvent::ClassicV3(module)),
        ))
    }

    fn visit_module_specification(
        &mut self,
        module: classic::package::ModuleSpecEntry<classic::Attrs>,
    ) -> Result<(), String> {
        self.accept(SemanticEvent::new(
            self.cursor
                .clone()
                .child(CursorSegment::Module(module.path.to_string())),
            SemanticEventKind::Module(ModuleEvent::ClassicV3Specification {
                path: module.path,
                specification: module.specification,
            }),
        ))
    }

    fn finish(self) -> Result<Self::Output, String> {
        if let Some(diagnostic) = self.failure {
            return Ok(Err(diagnostic));
        }
        Ok(self
            .sink
            .accept(SemanticEvent::new(self.cursor, SemanticEventKind::End))
            .and_then(|()| self.sink.finish()))
    }
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
            emit_classic_v3_dependencies(dependencies, &distribution_cursor, sink)?;
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
        classic::DistributionBody::Specs(package, dependencies, specification) => {
            sink.accept(SemanticEvent::new(
                distribution_cursor.clone(),
                SemanticEventKind::Begin(DistributionHeader::ClassicV3Specs { package }),
            ))?;
            emit_classic_v3_dependencies(dependencies, &distribution_cursor, sink)?;
            for module in specification.modules {
                let cursor = distribution_cursor
                    .clone()
                    .child(CursorSegment::Module(module.path.to_string()));
                sink.accept(SemanticEvent::new(
                    cursor,
                    SemanticEventKind::Module(ModuleEvent::ClassicV3Specification {
                        path: module.path,
                        specification: module.specification,
                    }),
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

/// Emits the dependencies of a Classic v3 distribution: their public faces.
fn emit_classic_v3_dependencies(
    dependencies: Vec<(classic::Path, classic::PackageSpecification<classic::Attrs>)>,
    parent: &IrCursor,
    sink: &mut dyn EventSink,
) -> Result<(), TransportDiagnostic> {
    for (dependency, specification) in dependencies {
        let cursor = parent
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
    Ok(())
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
            emit_v4_definition_dependencies(content.dependencies, &distribution_cursor, sink)?;
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

/// Emits the dependencies of a library or specification distribution: their public faces.
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

/// Emits an application's dependencies: the definitions it links statically
/// (distributions-0010).
fn emit_v4_definition_dependencies(
    dependencies: v4::DefinitionDependencies,
    parent: &IrCursor,
    sink: &mut dyn EventSink,
) -> Result<(), TransportDiagnostic> {
    for (package, definition) in dependencies {
        sink.accept(SemanticEvent::new(
            parent
                .clone()
                .child(CursorSegment::Dependency(package.clone())),
            SemanticEventKind::Dependency(DependencyEvent::V4Definition {
                package,
                definition,
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
            collect_classic_v3(source, ClassicKind::Library, package)
        }
        (IrVersion::V3, DistributionHeader::ClassicV3Specs { package }) => {
            collect_classic_v3(source, ClassicKind::Specs, package)
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

/// Which Classic v3 distribution header opened the event stream.
#[derive(Clone, Copy)]
enum ClassicKind {
    Library,
    Specs,
}

fn collect_classic_v3(
    source: &mut dyn EventSource,
    kind: ClassicKind,
    package: classic::Path,
) -> Result<SemanticFile, TransportDiagnostic> {
    let mut dependencies = Vec::new();
    let mut definitions = Vec::new();
    let mut specifications = Vec::new();
    while let Some(event) = source.next_event()? {
        let (cursor, event_kind) = event.into_parts();
        match (kind, event_kind) {
            (
                _,
                SemanticEventKind::Dependency(DependencyEvent::ClassicV3 {
                    package,
                    specification,
                }),
            ) => dependencies.push((package, specification)),
            (ClassicKind::Library, SemanticEventKind::Module(ModuleEvent::ClassicV3(module))) => {
                definitions.push(module)
            }
            (
                ClassicKind::Specs,
                SemanticEventKind::Module(ModuleEvent::ClassicV3Specification {
                    path,
                    specification,
                }),
            ) => specifications.push(classic::package::ModuleSpecEntry {
                path,
                specification,
            }),
            (ClassicKind::Specs, SemanticEventKind::Module(ModuleEvent::ClassicV3(_))) => {
                return Err(event_error(
                    "morphir::ir::codec::module_kind_mismatch",
                    Stage::Encoding,
                    cursor,
                    "a v3 Specs distribution received a module definition",
                ));
            }
            (
                ClassicKind::Library,
                SemanticEventKind::Module(ModuleEvent::ClassicV3Specification { .. }),
            ) => {
                return Err(event_error(
                    "morphir::ir::codec::module_kind_mismatch",
                    Stage::Encoding,
                    cursor,
                    "a v3 Library distribution received a module specification",
                ));
            }
            (_, SemanticEventKind::End) => {
                ensure_finished(source, cursor)?;
                let distribution = match kind {
                    ClassicKind::Library => classic::DistributionBody::Library(
                        package,
                        dependencies,
                        classic::PackageDefinition {
                            modules: definitions,
                        },
                    ),
                    ClassicKind::Specs => classic::DistributionBody::Specs(
                        package,
                        dependencies,
                        classic::PackageSpecification {
                            modules: specifications,
                        },
                    ),
                };
                return Ok(SemanticFile::ClassicV3(classic::Distribution {
                    format_version: 3,
                    distribution,
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
    let mut definition_dependencies = IndexMap::new();
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
            SemanticEventKind::Dependency(DependencyEvent::V4Definition {
                package,
                definition,
            }) => {
                definition_dependencies.insert(package, definition);
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
                    } if specifications.is_empty() && definition_dependencies.is_empty() => (
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
                    } if definitions.is_empty() && definition_dependencies.is_empty() => (
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
                    } if specifications.is_empty() && dependencies.is_empty() => (
                        format_version,
                        v4::Distribution::Application(v4::ApplicationContent {
                            package_name: package,
                            dependencies: definition_dependencies,
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
                            "v4 dependency or module events do not match the distribution kind",
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
