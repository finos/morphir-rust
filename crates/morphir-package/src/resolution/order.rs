use super::model::*;
use std::cmp::Ordering;

pub(super) fn release_entry(left: &ReleaseId, right: &ReleaseId) -> Ordering {
    left.package_path
        .cmp(&right.package_path)
        .then_with(|| right.version.cmp(&left.version))
}

pub(super) fn graph(left: &LockedGraph, right: &LockedGraph) -> Ordering {
    let mut left_releases: Vec<_> = left.nodes.iter().map(|node| &node.release).collect();
    let mut right_releases: Vec<_> = right.nodes.iter().map(|node| &node.release).collect();
    left_releases.sort_by(|left, right| release_entry(left, right));
    right_releases.sort_by(|left, right| release_entry(left, right));
    left_releases
        .iter()
        .zip(&right_releases)
        .find_map(|(left, right)| {
            let ordering = release_entry(left, right);
            ordering.ne(&Ordering::Equal).then_some(ordering)
        })
        .unwrap_or_else(|| left_releases.len().cmp(&right_releases.len()))
}

pub(super) fn normalize_graph(mut graph: LockedGraph) -> LockedGraph {
    for node in &mut graph.nodes {
        node.bindings
            .sort_by(|left, right| left.ir_package_name.cmp(&right.ir_package_name));
    }
    graph
        .nodes
        .sort_by(|left, right| node_order(&graph.root, left, right));
    graph
}

fn node_order(root: &ReleaseId, left: &LockedNode, right: &LockedNode) -> Ordering {
    (left.release != *root)
        .cmp(&(right.release != *root))
        .then_with(|| release_entry(&left.release, &right.release))
}

pub(super) fn normalize_record(mut record: ReleaseRecord) -> ReleaseRecord {
    record
        .dependencies
        .sort_by(|left, right| left.ir_package_name.cmp(&right.ir_package_name));
    record
}

pub(super) fn normalize_catalog(mut catalog: Catalog) -> Catalog {
    catalog.releases = catalog.releases.into_iter().map(normalize_record).collect();
    catalog
        .releases
        .sort_by(|left, right| right.release.version.cmp(&left.release.version));
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_order_is_reflexive_for_the_root_node() {
        let release = ReleaseId {
            package_path: PackagePath::parse("example.com/app/root").unwrap(),
            version: StableVersion::parse("1.0.0").unwrap(),
        };
        let candidate = LockedGraph {
            root: release.clone(),
            nodes: vec![LockedNode {
                release,
                ir_package_name: IrPackageName::parse("example/app").unwrap(),
                manifest_digest: ResolutionDigest::parse(&format!("sha256:{}", "0".repeat(64)))
                    .unwrap(),
                content_digest: ResolutionDigest::parse(&format!("sha256:{}", "0".repeat(64)))
                    .unwrap(),
                bindings: vec![],
            }],
        };
        assert_eq!(graph(&candidate, &candidate), Ordering::Equal);
        assert_eq!(
            node_order(&candidate.root, &candidate.nodes[0], &candidate.nodes[0]),
            Ordering::Equal
        );
        let normalized = normalize_graph(candidate.clone());
        assert_eq!(normalized, candidate);
    }
}
