//! Pure ordered configuration stages. Filesystem selection stays in the loader.
//!
//! Inputs are already parsed values. These stages preserve legacy policy; they
//! do not grant directory access or qualify byte, parser or allocation bounds.
//! Stage types keep early root decoding before member/user/environment layers.
use super::{
    ENVIRONMENT_DECLARING_PATH, LayerNotes, LayerScope, apply_ir_mode_alias, decode_config,
    take_workspace_out_dir,
};
use crate::config::{
    discovery::user_override_candidates,
    provenance::{ConfigOrigin, ConfigProvenance, ProvenanceState},
    sources::{ConfigSourceKind, SourceSelection},
};
use anyhow::Result;
use morphir_common::config::MorphirConfig;
use morphir_config::builtin_defaults;
use serde_json::Value;
use std::{
    marker::PhantomData,
    path::{Path, PathBuf},
};

pub(super) struct FileLayer {
    value: Value,
    declaring_path: PathBuf,
}

impl FileLayer {
    pub(super) fn new(value: Value, declaring_path: PathBuf) -> Self {
        Self {
            value,
            declaring_path,
        }
    }
}

pub(super) struct AwaitSystem;
pub(super) struct AwaitGlobal;
pub(super) struct AwaitProject;
pub(super) struct AwaitMember;
pub(super) struct AwaitUsers;

pub(super) struct LayerStack<Stage> {
    state: ProvenanceState,
    notes: LayerNotes,
    stage: PhantomData<Stage>,
}

impl<Stage> LayerStack<Stage> {
    fn next<Next>(self) -> LayerStack<Next> {
        LayerStack {
            state: self.state,
            notes: self.notes,
            stage: PhantomData,
        }
    }
}

impl LayerStack<AwaitSystem> {
    pub(super) fn new() -> Self {
        let mut state = ProvenanceState::default();
        state.merge(
            &builtin_defaults(),
            ConfigOrigin {
                kind: ConfigSourceKind::Defaults,
                path: None,
            },
        );
        Self {
            state,
            notes: LayerNotes::default(),
            stage: PhantomData,
        }
    }

    pub(super) fn system(mut self, layer: Option<FileLayer>) -> Result<LayerStack<AwaitGlobal>> {
        merge_file_layer(
            &mut self.state,
            layer,
            ConfigSourceKind::System,
            LayerScope::Workspace,
            &mut self.notes,
        )?;
        Ok(self.next())
    }
}
impl LayerStack<AwaitGlobal> {
    pub(super) fn global(mut self, layer: Option<FileLayer>) -> Result<LayerStack<AwaitProject>> {
        merge_file_layer(
            &mut self.state,
            layer,
            ConfigSourceKind::Global,
            LayerScope::Workspace,
            &mut self.notes,
        )?;
        Ok(self.next())
    }
}
impl LayerStack<AwaitProject> {
    pub(super) fn project(
        mut self,
        layer: Option<FileLayer>,
    ) -> Result<(MorphirConfig, LayerStack<AwaitMember>)> {
        merge_file_layer(
            &mut self.state,
            layer,
            ConfigSourceKind::Project,
            LayerScope::Workspace,
            &mut self.notes,
        )?;
        Ok((decode_config(self.state.value(), "project")?, self.next()))
    }
}

impl LayerStack<AwaitMember> {
    pub(super) fn member(mut self, layer: Option<FileLayer>) -> Result<LayerStack<AwaitUsers>> {
        merge_file_layer(
            &mut self.state,
            layer,
            ConfigSourceKind::WorkspaceMember,
            LayerScope::Member,
            &mut self.notes,
        )?;
        Ok(self.next())
    }
}

pub(super) struct MergedLayers {
    pub(super) value: Value,
    pub(super) provenance: ConfigProvenance,
    pub(super) notes: LayerNotes,
}

pub(super) enum EnvironmentLayer {
    Skipped,
    Selected(Value),
}

impl LayerStack<AwaitUsers> {
    pub(super) fn user(&mut self, layer: Option<FileLayer>, scope: LayerScope) -> Result<()> {
        merge_file_layer(
            &mut self.state,
            layer,
            ConfigSourceKind::UserOverride,
            scope,
            &mut self.notes,
        )
    }

    pub(super) fn environment(mut self, layer: EnvironmentLayer) -> Result<MergedLayers> {
        merge_environment_layer(&mut self.state, layer, &mut self.notes)?;
        let (value, provenance) = self.state.into_parts();
        Ok(MergedLayers {
            value,
            provenance,
            notes: self.notes,
        })
    }
}

pub(super) struct PlannedUserSource {
    pub(super) selection: SourceSelection,
    pub(super) candidates: Vec<PathBuf>,
    pub(super) scope: LayerScope,
}

pub(super) fn user_override_plan(
    selection: &SourceSelection,
    project_primary: Option<&Path>,
    member_primary: Option<&Path>,
) -> Vec<PlannedUserSource> {
    match selection {
        SourceSelection::Discover => {
            let mut plan = project_primary
                .into_iter()
                .map(|path| (path, LayerScope::Workspace))
                .chain(
                    member_primary
                        .filter(|member| Some(*member) != project_primary)
                        .map(|path| (path, LayerScope::Member)),
                )
                .filter_map(|(path, scope)| {
                    user_override_candidates(path).map(|candidates| PlannedUserSource {
                        selection: SourceSelection::Discover,
                        candidates: candidates.to_vec(),
                        scope,
                    })
                })
                .collect::<Vec<_>>();
            if plan.is_empty() {
                plan.push(PlannedUserSource {
                    selection: SourceSelection::Skip,
                    candidates: Vec::new(),
                    scope: LayerScope::Workspace,
                });
            }
            plan
        }
        SourceSelection::Explicit(_) | SourceSelection::Skip => vec![PlannedUserSource {
            selection: selection.clone(),
            candidates: Vec::new(),
            scope: LayerScope::Workspace,
        }],
    }
}

fn merge_environment_layer(
    state: &mut ProvenanceState,
    layer: EnvironmentLayer,
    notes: &mut LayerNotes,
) -> Result<()> {
    if let EnvironmentLayer::Selected(mut layer) = layer {
        if apply_ir_mode_alias(&mut layer, Path::new(ENVIRONMENT_DECLARING_PATH))? {
            notes.ir_mode_shadowed = true;
        }
        state.merge(
            &layer,
            ConfigOrigin {
                kind: ConfigSourceKind::Environment,
                path: None,
            },
        );
    }
    Ok(())
}

/// Prepare and merge one owned value, without accessing its declaring path.
///
/// Member primaries and adjacent member overrides cannot relocate the shared
/// output root. Remove that key before merging so both value and provenance
/// remain truthful. Normalize ir.mode in each layer before it meets any other
/// layer; normalizing the final value would reverse cross-layer precedence.
fn merge_file_layer(
    state: &mut ProvenanceState,
    layer: Option<FileLayer>,
    kind: ConfigSourceKind,
    scope: LayerScope,
    notes: &mut LayerNotes,
) -> Result<()> {
    if let Some(FileLayer {
        mut value,
        declaring_path,
    }) = layer
    {
        if scope == LayerScope::Member && take_workspace_out_dir(&mut value) {
            notes.out_dir.push(declaring_path.clone());
        }
        if apply_ir_mode_alias(&mut value, &declaring_path)? {
            notes.ir_mode_shadowed = true;
        }
        state.merge(
            &value,
            ConfigOrigin {
                kind,
                path: Some(declaring_path),
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
