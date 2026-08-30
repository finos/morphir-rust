use std::collections::{BTreeMap, BTreeSet};

use morphir_extension_sdk::Artifact;

use crate::AvroDiagnostic;

pub(super) fn detect_cycle(
    graph: &BTreeMap<String, BTreeSet<String>>,
    selected: &BTreeSet<String>,
) -> Result<(), AvroDiagnostic> {
    fn visit(
        name: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        selected: &BTreeSet<String>,
        visiting: &mut Vec<String>,
        visited: &mut BTreeSet<String>,
    ) -> Result<(), AvroDiagnostic> {
        if visited.contains(name) {
            return Ok(());
        }
        if let Some(start) = visiting.iter().position(|active| active == name) {
            let cycle = &visiting[start..];
            if cycle.len() > 1 {
                return Err(AvroDiagnostic::unsafe_recursion(cycle.join(" -> ")));
            }
            return Ok(());
        }
        visiting.push(name.to_owned());
        if let Some(dependencies) = graph.get(name) {
            for dependency in dependencies {
                if selected.contains(dependency) && dependency != name {
                    visit(dependency, graph, selected, visiting, visited)?;
                }
            }
        }
        visiting.pop();
        visited.insert(name.to_owned());
        Ok(())
    }

    let mut visiting = Vec::new();
    let mut visited = BTreeSet::new();
    for name in selected {
        visit(name, graph, selected, &mut visiting, &mut visited)?;
    }
    Ok(())
}

pub(super) fn text_artifact(path: String, content: String) -> Artifact {
    Artifact {
        path,
        content,
        binary: false,
    }
}

pub(super) fn insert_artifact(
    artifacts: &mut Vec<Artifact>,
    paths: &mut BTreeSet<String>,
    artifact: Artifact,
) -> Result<(), AvroDiagnostic> {
    if !paths.insert(artifact.path.clone()) {
        return Err(AvroDiagnostic::name_collision(format!(
            "duplicate artifact path {}",
            artifact.path
        )));
    }
    artifacts.push(artifact);
    Ok(())
}
