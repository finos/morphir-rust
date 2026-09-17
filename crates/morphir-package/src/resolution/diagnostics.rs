use super::execution::{Budget, release_record_clone_weight};
use super::model::*;
use super::order;
use super::search::SearchPolicy;
use super::update;
use super::validate::OuterInput;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Tree {
    record: ReleaseRecord,
    children: Vec<(IrPackageName, Tree)>,
}

pub(super) fn scope_conflict(
    input: &OuterInput,
    old: &LockedGraph,
    graphs: &[LockedGraph],
    budget: &mut Budget,
) -> Result<ResolutionDiagnostic, ResolutionExecutionError> {
    let mut best: Option<Witness> = None;
    for graph in graphs {
        let witness = graph_witness(graph, budget)?;
        if best
            .as_ref()
            .is_none_or(|current| witness_order(&witness, current).is_lt())
        {
            best = Some(witness);
        }
    }
    let witness = best.expect("scope conflict has a relaxed graph");
    Ok(ResolutionDiagnostic::UpdateScopeConflict {
        changed_pins: changed_pins(input, old, &witness),
        witness,
    })
}

pub(super) fn unsupported(
    input: &OuterInput,
    policy: &SearchPolicy,
    old: Option<&LockedGraph>,
    budget: &mut Budget,
) -> Result<Option<ResolutionDiagnostic>, ResolutionExecutionError> {
    let Some(witness) = abstract_witnesses(input, policy, budget)?
        .into_iter()
        .min_by(witness_order)
    else {
        return Ok(None);
    };
    let changed_pins = old.map_or_else(Vec::new, |old| changed_pins(input, old, &witness));
    Ok(Some(ResolutionDiagnostic::UnsupportedCapability {
        required_capabilities: vec![RequiredCapability::GraphAwareCoexistence],
        changed_pins,
        witness,
    }))
}

pub(super) fn unsatisfiable(
    input: &OuterInput,
    reachable_catalogs: &BTreeSet<PackagePath>,
) -> ResolutionDiagnostic {
    let mut catalogs: Vec<_> = input
        .catalogs
        .iter()
        .filter(|catalog| reachable_catalogs.contains(&catalog.value.package_path))
        .map(|catalog| order::normalize_catalog(catalog.value.clone()))
        .collect();
    catalogs.sort_by(|left, right| left.package_path.cmp(&right.package_path));
    let mut targets: Vec<_> = input
        .targets
        .iter()
        .map(|target| target.value.clone())
        .collect();
    targets.sort_by(|left, right| left.package_path().cmp(right.package_path()));
    ResolutionDiagnostic::UnsatisfiableRequirements {
        root: order::normalize_record(input.root.value.clone()),
        catalogs,
        targets,
    }
}

fn abstract_witnesses(
    input: &OuterInput,
    policy: &SearchPolicy,
    budget: &mut Budget,
) -> Result<Vec<Witness>, ResolutionExecutionError> {
    budget.charge(
        input.catalogs.len().saturating_add(1),
        "diagnostic catalog indexing",
    )?;
    let catalogs: BTreeMap<_, _> = input
        .catalogs
        .iter()
        .map(|catalog| (catalog.value.package_path.clone(), &catalog.value.releases))
        .collect();
    let trees = expand(
        &input.root.value,
        &catalogs,
        policy,
        &BTreeSet::from([input.root.value.release.clone()]),
        &input.root.value,
        budget,
    )?;
    let mut witnesses = Vec::new();
    for tree in trees {
        budget.charge(1, "diagnostic witness selection")?;
        if policy
            .required_targets
            .iter()
            .all(|path| tree_contains_path(&tree, path))
        {
            witnesses.push(tree_witness(&tree, budget)?);
        }
    }
    Ok(witnesses)
}

fn expand(
    record: &ReleaseRecord,
    catalogs: &BTreeMap<PackagePath, &Vec<ReleaseRecord>>,
    policy: &SearchPolicy,
    ancestors: &BTreeSet<ReleaseId>,
    root: &ReleaseRecord,
    budget: &mut Budget,
) -> Result<Vec<Tree>, ResolutionExecutionError> {
    budget.charge(1, "abstract diagnostic search")?;
    if ancestors.len() > 512 {
        return Err(ResolutionExecutionError::new(
            "diagnostic witness exceeds the 512-release execution budget",
        ));
    }
    budget.charge(
        record.dependencies.len(),
        "abstract diagnostic requirements",
    )?;
    let mut requirements = record.dependencies.clone();
    requirements.sort_by(|left, right| left.ir_package_name.cmp(&right.ir_package_name));
    let mut partials = vec![Vec::new()];
    for requirement in requirements {
        let candidate_count = catalogs
            .get(&requirement.package_path)
            .map_or(0, |releases| releases.len())
            .saturating_add(usize::from(
                requirement.package_path == root.release.package_path,
            ));
        budget.charge(candidate_count, "abstract diagnostic candidates")?;
        let candidates: Vec<&ReleaseRecord> =
            if requirement.package_path == root.release.package_path {
                std::iter::once(root)
                    .chain(
                        catalogs
                            .get(&requirement.package_path)
                            .into_iter()
                            .flat_map(|releases| releases.iter()),
                    )
                    .collect()
            } else {
                catalogs
                    .get(&requirement.package_path)
                    .into_iter()
                    .flat_map(|releases| releases.iter())
                    .collect()
            };
        let mut choices = Vec::new();
        for candidate in candidates {
            if candidate.ir_package_name != requirement.ir_package_name
                || !requirement
                    .version_range
                    .contains(&candidate.release.version)
                || policy
                    .exact
                    .get(&candidate.release.package_path)
                    .is_some_and(|version| version != &candidate.release.version)
                || ancestors.contains(&candidate.release)
            {
                continue;
            }
            budget.charge(
                ancestors.len().saturating_add(1),
                "abstract diagnostic ancestry",
            )?;
            let mut next_ancestors = ancestors.clone();
            next_ancestors.insert(candidate.release.clone());
            for tree in expand(candidate, catalogs, policy, &next_ancestors, root, budget)? {
                budget.charge(1, "abstract diagnostic choice")?;
                choices.push((requirement.ir_package_name.clone(), tree));
            }
        }
        if choices.is_empty() {
            return Ok(Vec::new());
        }
        partials = combine_partials(&partials, &choices, budget)?;
    }
    let mut trees = Vec::new();
    for children in partials {
        budget.charge(
            record.dependencies.len().saturating_add(1),
            "abstract diagnostic result",
        )?;
        trees.push(Tree {
            record: record.clone(),
            children,
        });
    }
    Ok(trees)
}

fn combine_partials(
    partials: &[Vec<(IrPackageName, Tree)>],
    choices: &[(IrPackageName, Tree)],
    budget: &mut Budget,
) -> Result<Vec<Vec<(IrPackageName, Tree)>>, ResolutionExecutionError> {
    let mut combined_partials = Vec::new();
    for partial in partials {
        for choice in choices {
            let clone_cost =
                children_clone_weight(partial).saturating_add(child_clone_weight(choice));
            budget.charge(clone_cost, "abstract diagnostic combination")?;
            let mut combined = partial.clone();
            combined.push(choice.clone());
            combined_partials.push(combined);
        }
    }
    Ok(combined_partials)
}

fn children_clone_weight(children: &[(IrPackageName, Tree)]) -> usize {
    children.iter().fold(0usize, |weight, child| {
        weight.saturating_add(child_clone_weight(child))
    })
}

fn child_clone_weight((_, tree): &(IrPackageName, Tree)) -> usize {
    1usize.saturating_add(tree_clone_weight(tree))
}

fn tree_clone_weight(tree: &Tree) -> usize {
    release_record_clone_weight(&tree.record).saturating_add(children_clone_weight(&tree.children))
}

fn tree_contains_path(tree: &Tree, path: &PackagePath) -> bool {
    &tree.record.release.package_path == path
        || tree
            .children
            .iter()
            .any(|(_, child)| tree_contains_path(child, path))
}

fn tree_witness(tree: &Tree, budget: &mut Budget) -> Result<Witness, ResolutionExecutionError> {
    let mut nodes = Vec::new();
    unfold_tree(tree, Vec::new(), &mut nodes, budget)?;
    nodes.sort_by(|left, right| left.occurrence.cmp(&right.occurrence));
    Ok(Witness { nodes })
}

fn unfold_tree(
    tree: &Tree,
    occurrence: Vec<IrPackageName>,
    nodes: &mut Vec<WitnessNode>,
    budget: &mut Budget,
) -> Result<(), ResolutionExecutionError> {
    budget.charge(
        occurrence.len().saturating_add(1).saturating_add(
            tree.children
                .len()
                .saturating_mul(occurrence.len().saturating_add(2)),
        ),
        "diagnostic witness unfolding",
    )?;
    let bindings = tree
        .children
        .iter()
        .map(|(name, _)| {
            let mut target_occurrence = occurrence.clone();
            target_occurrence.push(name.clone());
            WitnessBinding {
                ir_package_name: name.clone(),
                target_occurrence,
            }
        })
        .collect();
    nodes.push(WitnessNode {
        occurrence: occurrence.clone(),
        release: tree.record.release.clone(),
        bindings,
    });
    for (name, child) in &tree.children {
        let mut child_occurrence = occurrence.clone();
        child_occurrence.push(name.clone());
        unfold_tree(child, child_occurrence, nodes, budget)?;
    }
    Ok(())
}

fn graph_witness(
    graph: &LockedGraph,
    budget: &mut Budget,
) -> Result<Witness, ResolutionExecutionError> {
    budget.charge(graph.nodes.len(), "diagnostic graph indexing")?;
    let nodes: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.release.clone(), node))
        .collect();
    let mut witness_nodes = Vec::new();
    unfold_graph(&graph.root, Vec::new(), &nodes, &mut witness_nodes, budget)?;
    witness_nodes.sort_by(|left, right| left.occurrence.cmp(&right.occurrence));
    Ok(Witness {
        nodes: witness_nodes,
    })
}

fn unfold_graph(
    release: &ReleaseId,
    occurrence: Vec<IrPackageName>,
    nodes: &BTreeMap<ReleaseId, &LockedNode>,
    witness: &mut Vec<WitnessNode>,
    budget: &mut Budget,
) -> Result<(), ResolutionExecutionError> {
    let node = nodes[release];
    budget.charge(
        occurrence.len().saturating_add(1).saturating_add(
            node.bindings
                .len()
                .saturating_mul(occurrence.len().saturating_add(2)),
        ),
        "diagnostic witness unfolding",
    )?;
    let bindings: Vec<_> = node
        .bindings
        .iter()
        .map(|binding| {
            let mut target_occurrence = occurrence.clone();
            target_occurrence.push(binding.ir_package_name.clone());
            WitnessBinding {
                ir_package_name: binding.ir_package_name.clone(),
                target_occurrence,
            }
        })
        .collect();
    witness.push(WitnessNode {
        occurrence: occurrence.clone(),
        release: release.clone(),
        bindings,
    });
    for binding in &node.bindings {
        let mut child_occurrence = occurrence.clone();
        child_occurrence.push(binding.ir_package_name.clone());
        unfold_graph(&binding.target, child_occurrence, nodes, witness, budget)?;
    }
    Ok(())
}

fn witness_order(left: &Witness, right: &Witness) -> Ordering {
    let mut left_releases: Vec<_> = left.nodes.iter().map(|node| &node.release).collect();
    let mut right_releases: Vec<_> = right.nodes.iter().map(|node| &node.release).collect();
    left_releases.sort_by(|left, right| order::release_entry(left, right));
    right_releases.sort_by(|left, right| order::release_entry(left, right));
    compare_release_lists(&left_releases, &right_releases).then_with(|| {
        left.nodes
            .iter()
            .zip(&right.nodes)
            .find_map(|(left, right)| {
                let ordering = left
                    .occurrence
                    .cmp(&right.occurrence)
                    .then_with(|| order::release_entry(&left.release, &right.release))
                    .then_with(|| compare_bindings(&left.bindings, &right.bindings));
                ordering.ne(&Ordering::Equal).then_some(ordering)
            })
            .unwrap_or_else(|| left.nodes.len().cmp(&right.nodes.len()))
    })
}

fn compare_release_lists(left: &[&ReleaseId], right: &[&ReleaseId]) -> Ordering {
    left.iter()
        .zip(right)
        .find_map(|(left, right)| {
            let ordering = order::release_entry(left, right);
            ordering.ne(&Ordering::Equal).then_some(ordering)
        })
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn compare_bindings(left: &[WitnessBinding], right: &[WitnessBinding]) -> Ordering {
    left.iter()
        .zip(right)
        .find_map(|(left, right)| {
            let ordering = left
                .ir_package_name
                .cmp(&right.ir_package_name)
                .then_with(|| left.target_occurrence.cmp(&right.target_occurrence));
            ordering.ne(&Ordering::Equal).then_some(ordering)
        })
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn changed_pins(input: &OuterInput, old: &LockedGraph, witness: &Witness) -> Vec<ChangedPin> {
    let targets: BTreeSet<_> = input
        .targets
        .iter()
        .map(|target| target.value.package_path().clone())
        .collect();
    let closure = update::old_target_closure(old, &targets);
    let selected: BTreeSet<_> = witness
        .nodes
        .iter()
        .map(|node| node.release.clone())
        .collect();
    let mut changed = Vec::new();
    for node in old
        .nodes
        .iter()
        .filter(|node| !closure.contains(&node.release.package_path))
    {
        let mut replacements: Vec<_> = selected
            .iter()
            .filter(|release| {
                release.package_path == node.release.package_path && **release != node.release
            })
            .cloned()
            .collect();
        replacements.sort_by(|left, right| right.version.cmp(&left.version));
        if replacements.is_empty() {
            if !selected.contains(&node.release) {
                changed.push(ChangedPin::Removed {
                    previous: node.release.clone(),
                });
            }
        } else {
            changed.extend(
                replacements
                    .into_iter()
                    .map(|selected| ChangedPin::Changed {
                        previous: node.release.clone(),
                        selected,
                    }),
            );
        }
    }
    changed.sort_by(|left, right| {
        let (left_previous, left_selected) = pin_parts(left);
        let (right_previous, right_selected) = pin_parts(right);
        left_previous
            .package_path
            .cmp(&right_previous.package_path)
            .then_with(|| match (left_selected, right_selected) {
                (Some(left), Some(right)) => right.version.cmp(&left.version),
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (None, None) => Ordering::Equal,
            })
    });
    changed
}

fn pin_parts(pin: &ChangedPin) -> (&ReleaseId, Option<&ReleaseId>) {
    match pin {
        ChangedPin::Changed { previous, selected } => (previous, Some(selected)),
        ChangedPin::Removed { previous } => (previous, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolution::execution::Budget;

    #[test]
    fn dense_diamond_witness_stops_at_the_aggregate_budget() {
        let graph = diamond_graph(5);
        assert_eq!(graph.nodes.len(), 13);
        let mut budget = Budget::with_limit(64);
        assert!(graph_witness(&graph, &mut budget).is_err());
    }

    #[test]
    fn abstract_expansion_checks_the_budget_before_pushing_a_result() {
        let root = release_record("root", "example/root");
        let mut budget = Budget::with_limit(1);
        assert!(
            expand(
                &root,
                &BTreeMap::new(),
                &SearchPolicy::default(),
                &BTreeSet::from([root.release.clone()]),
                &root,
                &mut budget,
            )
            .is_err()
        );
    }

    #[test]
    fn cartesian_expansion_charges_deep_partial_clone_weight() {
        let leaf = Tree {
            record: release_record("leaf", "example/leaf"),
            children: Vec::new(),
        };
        let deep = Tree {
            record: release_record("deep", "example/deep"),
            children: vec![(IrPackageName::parse("example/leaf").unwrap(), leaf.clone())],
        };
        let partials = vec![vec![(IrPackageName::parse("example/deep").unwrap(), deep)]];
        let choices = vec![(IrPackageName::parse("example/choice").unwrap(), leaf)];
        let mut budget = Budget::with_limit(3);
        assert!(combine_partials(&partials, &choices, &mut budget).is_err());
    }

    fn diamond_graph(layers: usize) -> LockedGraph {
        let root = release_id("root");
        let mut nodes = vec![locked_node(root.clone(), "example/root", Vec::new())];
        for layer in 0..=layers {
            for side in ["a", "b"] {
                nodes.push(locked_node(
                    release_id(&format!("{side}{layer}")),
                    &format!("example/{side}{layer}"),
                    Vec::new(),
                ));
            }
        }
        nodes[0].bindings = vec![binding("example/a0", "a0"), binding("example/b0", "b0")];
        for layer in 0..layers {
            for side in ["a", "b"] {
                let release = release_id(&format!("{side}{layer}"));
                let node = nodes
                    .iter_mut()
                    .find(|node| node.release == release)
                    .unwrap();
                node.bindings = vec![
                    binding(
                        &format!("example/a{}", layer + 1),
                        &format!("a{}", layer + 1),
                    ),
                    binding(
                        &format!("example/b{}", layer + 1),
                        &format!("b{}", layer + 1),
                    ),
                ];
            }
        }
        LockedGraph { root, nodes }
    }

    fn release_id(name: &str) -> ReleaseId {
        ReleaseId {
            package_path: PackagePath::parse(&format!("example.com/pkg/{name}")).unwrap(),
            version: StableVersion::parse("1.0.0").unwrap(),
        }
    }

    fn locked_node(release: ReleaseId, name: &str, bindings: Vec<Binding>) -> LockedNode {
        LockedNode {
            release,
            ir_package_name: IrPackageName::parse(name).unwrap(),
            manifest_digest: digest(),
            content_digest: digest(),
            bindings,
        }
    }

    fn release_record(name: &str, ir_name: &str) -> ReleaseRecord {
        ReleaseRecord {
            release: release_id(name),
            ir_package_name: IrPackageName::parse(ir_name).unwrap(),
            manifest_digest: digest(),
            content_digest: digest(),
            dependencies: Vec::new(),
        }
    }

    fn binding(name: &str, target: &str) -> Binding {
        Binding {
            ir_package_name: IrPackageName::parse(name).unwrap(),
            target: release_id(target),
        }
    }

    fn digest() -> ResolutionDigest {
        ResolutionDigest::parse(&format!("sha256:{}", "0".repeat(64))).unwrap()
    }
}
