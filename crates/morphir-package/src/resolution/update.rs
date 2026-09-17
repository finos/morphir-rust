use super::model::*;
use super::order;
use super::search::SearchPolicy;
use super::validate::{OuterInput, normalize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) fn target_violations(input: &OuterInput, old: &LockedGraph) -> Vec<Violation> {
    let old_paths: BTreeSet<_> = old
        .nodes
        .iter()
        .map(|node| node.release.package_path.clone())
        .collect();
    normalize(
        input
            .targets
            .iter()
            .filter(|target| {
                target.value.package_path() == &input.root.value.release.package_path
                    || !old_paths.contains(target.value.package_path())
            })
            .map(|target| Violation {
                pointer: format!("{}/packagePath", target.pointer),
                rule: ViolationRule::IdentityMismatch,
            })
            .collect(),
    )
}

pub(super) fn policy(input: &OuterInput, old: &LockedGraph, scoped: bool) -> SearchPolicy {
    let target_paths: BTreeSet<_> = input
        .targets
        .iter()
        .map(|target| target.value.package_path().clone())
        .collect();
    let closure = old_target_closure(old, &target_paths);
    let exact = input
        .targets
        .iter()
        .filter_map(|target| match &target.value {
            UpdateTarget::Exact {
                package_path,
                version,
            } => Some((package_path.clone(), version.clone())),
            UpdateTarget::Eligible { .. } => None,
        })
        .collect();
    let pins = if scoped {
        old.nodes
            .iter()
            .filter(|node| !closure.contains(&node.release.package_path))
            .map(|node| (node.release.package_path.clone(), node.release.clone()))
            .collect()
    } else {
        BTreeMap::new()
    };
    SearchPolicy {
        exact,
        pins,
        required_targets: target_paths,
    }
}

pub(super) fn choose(
    mut graphs: Vec<LockedGraph>,
    input: &OuterInput,
    old: &LockedGraph,
) -> Option<LockedGraph> {
    let mut targets: Vec<_> = input
        .targets
        .iter()
        .map(|target| target.value.package_path().clone())
        .collect();
    targets.sort();
    let target_set: BTreeSet<_> = targets.iter().cloned().collect();
    graphs.sort_by(|left, right| {
        target_versions(left, &targets)
            .iter()
            .zip(target_versions(right, &targets))
            .find_map(|(left, right)| {
                let ordering = right.cmp(left);
                ordering.ne(&Ordering::Equal).then_some(ordering)
            })
            .unwrap_or_else(|| {
                changed_count(left, old, &target_set)
                    .cmp(&changed_count(right, old, &target_set))
                    .then_with(|| order::graph(left, right))
            })
    });
    graphs.into_iter().next()
}

pub(super) fn old_target_closure(
    old: &LockedGraph,
    targets: &BTreeSet<PackagePath>,
) -> BTreeSet<PackagePath> {
    let by_path: BTreeMap<_, _> = old
        .nodes
        .iter()
        .map(|node| (node.release.package_path.clone(), node))
        .collect();
    let by_release: BTreeMap<_, _> = old
        .nodes
        .iter()
        .map(|node| (node.release.clone(), node))
        .collect();
    let mut closure = BTreeSet::new();
    let mut queue: VecDeque<_> = targets
        .iter()
        .filter_map(|path| by_path.get(path).map(|node| node.release.clone()))
        .collect();
    while let Some(release) = queue.pop_front() {
        if !closure.insert(release.package_path.clone()) {
            continue;
        }
        queue.extend(
            by_release[&release]
                .bindings
                .iter()
                .map(|binding| binding.target.clone()),
        );
    }
    closure
}

fn target_versions<'a>(graph: &'a LockedGraph, targets: &[PackagePath]) -> Vec<&'a StableVersion> {
    let selected: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (&node.release.package_path, &node.release.version))
        .collect();
    targets.iter().map(|path| selected[path]).collect()
}

fn changed_count(graph: &LockedGraph, old: &LockedGraph, targets: &BTreeSet<PackagePath>) -> usize {
    let selected: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (&node.release.package_path, &node.release.version))
        .collect();
    old.nodes
        .iter()
        .filter(|node| !targets.contains(&node.release.package_path))
        .filter(|node| {
            selected
                .get(&node.release.package_path)
                .is_none_or(|version| *version != &node.release.version)
        })
        .count()
}
