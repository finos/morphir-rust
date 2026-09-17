use super::super::model::ReleaseId;
use std::collections::{BTreeMap, BTreeSet};

/// Strongly connected component labels for a validated lock graph.
///
/// The two graph traversals are iterative so a deep replay graph cannot exhaust
/// the process stack. Domain identities remain the representation at this
/// private boundary; integer labels only compare already-discovered components.
pub(super) struct Components {
    labels: BTreeMap<ReleaseId, usize>,
    cyclic: BTreeSet<usize>,
}

impl Components {
    pub(super) fn analyze<'a>(
        nodes: impl IntoIterator<Item = &'a ReleaseId>,
        edges: impl IntoIterator<Item = (&'a ReleaseId, &'a ReleaseId)>,
    ) -> Self {
        let nodes: BTreeSet<_> = nodes.into_iter().cloned().collect();
        let mut adjacency: BTreeMap<_, Vec<_>> = nodes
            .iter()
            .cloned()
            .map(|node| (node, Vec::new()))
            .collect();
        let mut reverse = adjacency.clone();
        for (source, target) in edges {
            adjacency
                .entry(source.clone())
                .or_default()
                .push(target.clone());
            reverse
                .entry(target.clone())
                .or_default()
                .push(source.clone());
        }

        let finish_order = finish_order(&nodes, &adjacency);
        let (labels, sizes) = label_components(finish_order, &reverse);
        let mut cyclic = BTreeSet::new();
        for (source, targets) in &adjacency {
            let source_label = labels[source];
            for target in targets {
                if source_label == labels[target] && (source == target || sizes[source_label] > 1) {
                    cyclic.insert(source_label);
                }
            }
        }
        Self { labels, cyclic }
    }

    pub(super) fn is_cycle_edge(&self, source: &ReleaseId, target: &ReleaseId) -> bool {
        self.labels.get(source).is_some_and(|source_label| {
            self.labels.get(target) == Some(source_label) && self.cyclic.contains(source_label)
        })
    }
}

fn finish_order(
    nodes: &BTreeSet<ReleaseId>,
    adjacency: &BTreeMap<ReleaseId, Vec<ReleaseId>>,
) -> Vec<ReleaseId> {
    let mut visited = BTreeSet::new();
    let mut finished = Vec::with_capacity(nodes.len());
    for start in nodes {
        if !visited.insert(start.clone()) {
            continue;
        }
        let mut stack = vec![(start.clone(), 0usize)];
        while !stack.is_empty() {
            let next = {
                let (current, next_index) = stack.last_mut().expect("stack is not empty");
                let next = adjacency[current].get(*next_index).cloned();
                *next_index = next_index.saturating_add(1);
                next
            };
            match next {
                Some(target) if visited.insert(target.clone()) => stack.push((target, 0)),
                Some(_) => {}
                None => {
                    let (current, _) = stack.pop().expect("stack is not empty");
                    finished.push(current);
                }
            }
        }
    }
    finished
}

fn label_components(
    finish_order: Vec<ReleaseId>,
    reverse: &BTreeMap<ReleaseId, Vec<ReleaseId>>,
) -> (BTreeMap<ReleaseId, usize>, Vec<usize>) {
    let mut labels = BTreeMap::new();
    let mut sizes = Vec::new();
    for start in finish_order.into_iter().rev() {
        if labels.contains_key(&start) {
            continue;
        }
        let label = sizes.len();
        labels.insert(start.clone(), label);
        let mut size = 0usize;
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            size = size.saturating_add(1);
            for source in &reverse[&current] {
                if !labels.contains_key(source) {
                    labels.insert(source.clone(), label);
                    stack.push(source.clone());
                }
            }
        }
        sizes.push(size);
    }
    (labels, sizes)
}

#[cfg(test)]
mod tests {
    use super::Components;
    use crate::resolution::{PackagePath, ReleaseId, StableVersion};
    use std::collections::BTreeMap;

    #[test]
    fn labels_all_internal_cycle_edges_and_self_loops() {
        let a = release("a");
        let b = release("b");
        let c = release("c");
        let d = release("d");
        let nodes = [a.clone(), b.clone(), c.clone(), d.clone()];
        let edges = BTreeMap::from([
            (a.clone(), vec![b.clone()]),
            (b.clone(), vec![a.clone(), c.clone()]),
            (c.clone(), vec![b.clone()]),
            (d.clone(), vec![d.clone()]),
        ]);
        let components = Components::analyze(nodes.iter(), edge_pairs(&edges));

        assert!(components.is_cycle_edge(&a, &b));
        assert!(components.is_cycle_edge(&b, &a));
        assert!(components.is_cycle_edge(&b, &c));
        assert!(components.is_cycle_edge(&c, &b));
        assert!(components.is_cycle_edge(&d, &d));
        assert!(!components.is_cycle_edge(&a, &d));
    }

    #[test]
    fn analyzes_a_deep_acyclic_graph_iteratively() {
        let nodes: Vec<_> = (0..2_048)
            .map(|index| release(&format!("p{index}")))
            .collect();
        let edges: BTreeMap<_, _> = nodes
            .windows(2)
            .map(|pair| (pair[0].clone(), vec![pair[1].clone()]))
            .collect();
        let components = Components::analyze(nodes.iter(), edge_pairs(&edges));

        assert!(
            nodes
                .windows(2)
                .all(|pair| !components.is_cycle_edge(&pair[0], &pair[1]))
        );
    }

    fn edge_pairs(
        edges: &BTreeMap<ReleaseId, Vec<ReleaseId>>,
    ) -> impl Iterator<Item = (&ReleaseId, &ReleaseId)> {
        edges
            .iter()
            .flat_map(|(source, targets)| targets.iter().map(move |target| (source, target)))
    }

    fn release(name: &str) -> ReleaseId {
        ReleaseId {
            package_path: PackagePath::parse(&format!("example.com/pkg/{name}")).unwrap(),
            version: StableVersion::parse("1.0.0").unwrap(),
        }
    }
}
