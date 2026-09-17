use super::super::model::*;
use super::super::validate::{OuterInput, normalize};
use super::components::Components;
use super::{LocatedLock, violation};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) fn lock_topology(input: &OuterInput, lock: &LocatedLock) -> Vec<Violation> {
    let mut violations = Vec::new();
    let nodes: BTreeMap<_, _> = lock
        .nodes
        .iter()
        .map(|node| (node.value.release.clone(), node))
        .collect();
    let root_matches = lock.value.root == input.root.value.release;
    if !root_matches {
        violations.push(violation("/lock/root", ViolationRule::IdentityMismatch));
    }
    let root_present = nodes.contains_key(&lock.value.root);
    if root_matches && !root_present {
        violations.push(violation("/lock/root", ViolationRule::MissingRoot));
    }

    let mut edges: BTreeMap<ReleaseId, Vec<(ReleaseId, String)>> = BTreeMap::new();
    for node in &lock.nodes {
        for (index, binding) in node.value.bindings.iter().enumerate() {
            let pointer = format!("{}/bindings/{index}/target", node.pointer);
            if nodes.contains_key(&binding.target) {
                edges
                    .entry(node.value.release.clone())
                    .or_default()
                    .push((binding.target.clone(), pointer));
            } else {
                violations.push(violation(pointer, ViolationRule::DanglingBinding));
            }
        }
    }

    if root_matches && root_present {
        let mut reachable = BTreeSet::new();
        let mut queue = VecDeque::from([lock.value.root.clone()]);
        while let Some(release) = queue.pop_front() {
            if !reachable.insert(release.clone()) {
                continue;
            }
            queue.extend(
                edges
                    .get(&release)
                    .into_iter()
                    .flatten()
                    .map(|(target, _)| target.clone()),
            );
        }
        for node in &lock.nodes {
            if !reachable.contains(&node.value.release) {
                violations.push(violation(
                    format!("{}/release", node.pointer),
                    ViolationRule::UnreachableNode,
                ));
            }
        }
    }

    let components = Components::analyze(
        nodes.keys(),
        edges.iter().flat_map(|(source, outgoing)| {
            outgoing.iter().map(move |(target, _)| (source, target))
        }),
    );
    for (source, outgoing) in &edges {
        for (target, pointer) in outgoing {
            if components.is_cycle_edge(source, target) {
                violations.push(violation(pointer.clone(), ViolationRule::Cycle));
            }
        }
    }
    normalize(violations)
}
