use super::execution::{
    Budget, release_id_clone_weight, release_record_clone_weight, requirement_clone_weight,
};
use super::model::*;
use super::order;
use super::validate::OuterInput;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_SELECTED_RELEASES: usize = 512;

#[derive(Debug, Clone, Default)]
pub(super) struct SearchPolicy {
    pub(super) exact: BTreeMap<PackagePath, StableVersion>,
    pub(super) pins: BTreeMap<PackagePath, ReleaseId>,
    pub(super) required_targets: BTreeSet<PackagePath>,
}

#[derive(Clone)]
struct Pending {
    requirement: Requirement,
}

#[derive(Clone)]
struct State {
    selected: BTreeMap<PackagePath, ReleaseRecord>,
    names: BTreeMap<IrPackageName, ReleaseId>,
    pending: Vec<Pending>,
}

pub(super) fn complete_catalog_paths(
    input: &OuterInput,
) -> Result<BTreeSet<PackagePath>, Vec<MissingItem>> {
    let catalogs: BTreeMap<_, _> = input
        .catalogs
        .iter()
        .map(|catalog| (catalog.value.package_path.clone(), &catalog.value))
        .collect();
    let mut paths = BTreeSet::new();
    let mut missing = BTreeSet::new();
    let mut queue: VecDeque<_> = input
        .root
        .value
        .dependencies
        .iter()
        .map(|requirement| requirement.package_path.clone())
        .collect();
    while let Some(path) = queue.pop_front() {
        if !paths.insert(path.clone()) {
            continue;
        }
        if path == input.root.value.release.package_path && !catalogs.contains_key(&path) {
            continue;
        }
        let Some(catalog) = catalogs.get(&path) else {
            missing.insert(path);
            continue;
        };
        queue.extend(
            catalog
                .releases
                .iter()
                .flat_map(|release| &release.dependencies)
                .map(|requirement| requirement.package_path.clone()),
        );
    }
    if missing.is_empty() {
        Ok(paths)
    } else {
        Err(missing
            .into_iter()
            .map(|package_path| MissingItem::Catalog { package_path })
            .collect())
    }
}

pub(super) fn supported(
    input: &OuterInput,
    policy: &SearchPolicy,
    budget: &mut Budget,
) -> Result<Vec<LockedGraph>, ResolutionExecutionError> {
    budget.charge(
        input
            .catalogs
            .len()
            .saturating_add(input.root.value.dependencies.len())
            .saturating_add(3),
        "resolution search initialization",
    )?;
    let catalogs: BTreeMap<_, _> = input
        .catalogs
        .iter()
        .map(|catalog| (catalog.value.package_path.clone(), &catalog.value.releases))
        .collect();
    let root = input.root.value.clone();
    let state = State {
        selected: BTreeMap::from([(root.release.package_path.clone(), root.clone())]),
        names: BTreeMap::from([(root.ir_package_name.clone(), root.release.clone())]),
        pending: root
            .dependencies
            .iter()
            .cloned()
            .map(|requirement| Pending { requirement })
            .collect(),
    };
    let mut graphs = Vec::new();
    visit(state, &catalogs, policy, &root.release, &mut graphs, budget)?;
    graphs.sort_by(order::graph);
    graphs.dedup();
    Ok(graphs)
}

fn visit(
    mut state: State,
    catalogs: &BTreeMap<PackagePath, &Vec<ReleaseRecord>>,
    policy: &SearchPolicy,
    root: &ReleaseId,
    graphs: &mut Vec<LockedGraph>,
    budget: &mut Budget,
) -> Result<(), ResolutionExecutionError> {
    budget.charge(1, "resolution search")?;
    if state.selected.len() > MAX_SELECTED_RELEASES {
        return Err(ResolutionExecutionError::new(format!(
            "resolution graph exceeds the {MAX_SELECTED_RELEASES}-release execution budget"
        )));
    }
    let pending = loop {
        let Some(pending) = state.pending.pop() else {
            if policy
                .required_targets
                .iter()
                .all(|path| state.selected.contains_key(path))
            {
                let graph_cost = state.selected.len()
                    + state
                        .selected
                        .values()
                        .map(|record| record.dependencies.len())
                        .sum::<usize>();
                budget.charge(graph_cost, "resolution graph materialization")?;
                if let Some(graph) = build_graph(root, &state.selected) {
                    graphs.push(order::normalize_graph(graph));
                }
            }
            return Ok(());
        };
        budget.charge(1, "resolution requirement traversal")?;
        let Some(selected) = state.selected.get(&pending.requirement.package_path) else {
            break pending;
        };
        if selected.ir_package_name != pending.requirement.ir_package_name
            || !pending
                .requirement
                .version_range
                .contains(&selected.release.version)
        {
            return Ok(());
        }
    };

    let Some(candidates) = catalogs.get(&pending.requirement.package_path) else {
        return Ok(());
    };
    for candidate in *candidates {
        if !eligible(candidate, &pending.requirement, policy) {
            continue;
        }
        if let Some(selected) = state.names.get(&candidate.ir_package_name)
            && selected != &candidate.release
        {
            continue;
        }
        let clone_cost = state_clone_weight(&state, candidate);
        budget.charge(clone_cost, "resolution search branching")?;
        let mut branch = state.clone();
        branch
            .selected
            .insert(candidate.release.package_path.clone(), candidate.clone());
        branch
            .names
            .insert(candidate.ir_package_name.clone(), candidate.release.clone());
        branch.pending.extend(
            candidate
                .dependencies
                .iter()
                .cloned()
                .map(|requirement| Pending { requirement }),
        );
        visit(branch, catalogs, policy, root, graphs, budget)?;
    }
    Ok(())
}

fn state_clone_weight(state: &State, candidate: &ReleaseRecord) -> usize {
    let selected_weight = state.selected.values().fold(0usize, |weight, record| {
        weight
            .saturating_add(1) // BTreeMap package-path key.
            .saturating_add(release_record_clone_weight(record))
    });
    let names_weight = state.names.values().fold(0usize, |weight, release| {
        weight
            .saturating_add(1) // BTreeMap IR-name key.
            .saturating_add(release_id_clone_weight(release))
    });
    let pending_weight = state.pending.iter().fold(0usize, |weight, pending| {
        weight.saturating_add(requirement_clone_weight(&pending.requirement))
    });
    let candidate_insertion_weight =
        2usize // Selected path key and names IR key.
            .saturating_add(release_record_clone_weight(candidate))
            .saturating_add(release_id_clone_weight(&candidate.release))
            .saturating_add(
                candidate
                    .dependencies
                    .iter()
                    .fold(0usize, |weight, dependency| {
                        weight.saturating_add(requirement_clone_weight(dependency))
                    }),
            );
    selected_weight
        .saturating_add(names_weight)
        .saturating_add(pending_weight)
        .saturating_add(candidate_insertion_weight)
}

fn eligible(candidate: &ReleaseRecord, requirement: &Requirement, policy: &SearchPolicy) -> bool {
    candidate.ir_package_name == requirement.ir_package_name
        && requirement
            .version_range
            .contains(&candidate.release.version)
        && policy
            .exact
            .get(&candidate.release.package_path)
            .is_none_or(|version| version == &candidate.release.version)
        && policy
            .pins
            .get(&candidate.release.package_path)
            .is_none_or(|release| release == &candidate.release)
}

fn build_graph(
    root: &ReleaseId,
    selected: &BTreeMap<PackagePath, ReleaseRecord>,
) -> Option<LockedGraph> {
    let mut nodes = Vec::new();
    for record in selected.values() {
        let bindings: Vec<_> = record
            .dependencies
            .iter()
            .map(|requirement| {
                selected
                    .get(&requirement.package_path)
                    .map(|target| Binding {
                        ir_package_name: requirement.ir_package_name.clone(),
                        target: target.release.clone(),
                    })
            })
            .collect::<Option<_>>()?;
        nodes.push(LockedNode {
            release: record.release.clone(),
            ir_package_name: record.ir_package_name.clone(),
            manifest_digest: record.manifest_digest.clone(),
            content_digest: record.content_digest.clone(),
            bindings,
        });
    }
    let graph = LockedGraph {
        root: root.clone(),
        nodes,
    };
    acyclic(&graph).then_some(graph)
}

fn acyclic(graph: &LockedGraph) -> bool {
    let edges: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| {
            (
                node.release.clone(),
                node.bindings
                    .iter()
                    .map(|binding| binding.target.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    fn visit(
        node: &ReleaseId,
        edges: &BTreeMap<ReleaseId, Vec<ReleaseId>>,
        visiting: &mut BTreeSet<ReleaseId>,
        complete: &mut BTreeSet<ReleaseId>,
    ) -> bool {
        if complete.contains(node) {
            return true;
        }
        if !visiting.insert(node.clone()) {
            return false;
        }
        if !edges
            .get(node)
            .into_iter()
            .flatten()
            .all(|target| visit(target, edges, visiting, complete))
        {
            return false;
        }
        visiting.remove(node);
        complete.insert(node.clone());
        true
    }
    visit(
        &graph.root,
        &edges,
        &mut BTreeSet::new(),
        &mut BTreeSet::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolution::validate::{outer_identities, outer_shape};
    use serde_json::json;

    #[test]
    fn search_checks_the_budget_before_cloning_a_branch() {
        let digest = format!("sha256:{}", "0".repeat(64));
        let value = json!({
            "formatVersion": "0.1.0-draft.2",
            "capability": "flat-library",
            "mode": "initial",
            "root": {
                "release": { "packagePath": "example.com/app/root", "version": "1.0.0" },
                "irPackageName": "example/app",
                "manifestDigest": digest,
                "contentDigest": digest,
                "dependencies": [{
                    "irPackageName": "example/a",
                    "packagePath": "example.com/pkg/a",
                    "versionRange": { "minimumInclusive": "1.0.0", "maximumExclusive": "2.0.0" }
                }]
            },
            "catalogs": [{
                "packagePath": "example.com/pkg/a",
                "releases": [{
                    "release": { "packagePath": "example.com/pkg/a", "version": "1.0.0" },
                    "irPackageName": "example/a",
                    "manifestDigest": digest,
                    "contentDigest": digest,
                    "dependencies": []
                }]
            }]
        });
        let mut input = outer_shape(&value).unwrap();
        assert!(outer_identities(&mut input).is_empty());
        let mut budget = Budget::with_limit(7);
        assert!(supported(&input, &SearchPolicy::default(), &mut budget).is_err());
    }

    #[test]
    fn state_clone_weight_includes_selected_record_dependencies() {
        let digest = ResolutionDigest::parse(&format!("sha256:{}", "0".repeat(64))).unwrap();
        let dependency = Requirement {
            ir_package_name: IrPackageName::parse("example/a").unwrap(),
            package_path: PackagePath::parse("example.com/pkg/a").unwrap(),
            version_range: VersionRange {
                minimum_inclusive: StableVersion::parse("1.0.0").unwrap(),
                maximum_exclusive: StableVersion::parse("2.0.0").unwrap(),
            },
        };
        let record = ReleaseRecord {
            release: ReleaseId {
                package_path: PackagePath::parse("example.com/app/root").unwrap(),
                version: StableVersion::parse("1.0.0").unwrap(),
            },
            ir_package_name: IrPackageName::parse("example/app").unwrap(),
            manifest_digest: digest.clone(),
            content_digest: digest,
            dependencies: vec![dependency.clone(), dependency],
        };
        let state = State {
            selected: BTreeMap::from([(record.release.package_path.clone(), record.clone())]),
            names: BTreeMap::from([(record.ir_package_name.clone(), record.release.clone())]),
            pending: Vec::new(),
        };
        assert!(state_clone_weight(&state, &record) > state.selected.len() + state.names.len());
    }
}
