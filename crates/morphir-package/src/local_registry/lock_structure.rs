use super::*;
use std::collections::{HashMap, HashSet};

pub(super) fn identities(graph: &LockedGraph, s: &mut Shape) {
    let (mut releases, mut paths, mut names) = (HashSet::new(), HashSet::new(), HashSet::new());
    for (i, node) in graph.nodes().iter().enumerate() {
        let p = format!("/graph/nodes/{i}");
        if !releases.insert(node.release()) {
            s.add(format!("{p}/release"), Rule::DuplicateIdentity);
            continue;
        }
        if !paths.insert(node.release().package_path()) {
            s.add(format!("{p}/release/packagePath"), Rule::DuplicateIdentity);
            continue;
        }
        if !names.insert(node.ir_package_name()) {
            s.add(format!("{p}/irPackageName"), Rule::UnsupportedFlatBinding);
            continue;
        }
        unique(
            node.bindings().iter().enumerate().map(|(i, b)| {
                (
                    format!("{p}/bindings/{i}/irPackageName"),
                    b.ir_package_name().as_str(),
                )
            }),
            s,
        );
    }
}
pub(super) fn topology(graph: &LockedGraph, s: &mut Shape) {
    let nodes: HashMap<_, _> = graph
        .nodes()
        .iter()
        .enumerate()
        .map(|(i, n)| (n.release(), i))
        .collect();
    let root = nodes.get(graph.root()).copied();
    if root.is_none() {
        s.add("/graph/root", Rule::MissingRoot)
    }
    let edges: Vec<Vec<Option<usize>>> = graph
        .nodes()
        .iter()
        .map(|n| {
            n.bindings()
                .iter()
                .map(|b| nodes.get(b.target()).copied())
                .collect()
        })
        .collect();
    let mut reachable = HashSet::new();
    let mut pending: Vec<_> = root.into_iter().collect();
    while let Some(i) = pending.pop() {
        if reachable.insert(i) {
            pending.extend(edges[i].iter().flatten().copied())
        }
    }
    let components = components(&edges);
    for (i, node) in graph.nodes().iter().enumerate() {
        if root.is_some() && !reachable.contains(&i) {
            s.add(format!("/graph/nodes/{i}/release"), Rule::UnreachableNode)
        }
        for (j, _) in node.bindings().iter().enumerate() {
            let p = format!("/graph/nodes/{i}/bindings/{j}/target");
            match edges[i][j] {
                None => s.add(p, Rule::DanglingBinding),
                Some(target) if components[i] == components[target] => s.add(p, Rule::Cycle),
                _ => {}
            }
        }
    }
}
// Two iterative traversals label strongly connected components in O(nodes + edges).
fn components(edges: &[Vec<Option<usize>>]) -> Vec<usize> {
    let n = edges.len();
    let mut reverse = vec![vec![]; n];
    for (i, targets) in edges.iter().enumerate() {
        for target in targets.iter().flatten() {
            reverse[*target].push(i)
        }
    }
    let mut visited = vec![false; n];
    let mut order = Vec::new();
    for root in 0..n {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        let mut stack = vec![(root, 0)];
        while let Some((node, index)) = stack.last_mut() {
            if *index == edges[*node].len() {
                let (node, _) = stack.pop().unwrap();
                order.push(node);
                continue;
            }
            let target = edges[*node][*index];
            *index += 1;
            if let Some(target) = target
                && !visited[target]
            {
                visited[target] = true;
                stack.push((target, 0))
            }
        }
    }
    let mut labels = vec![None; n];
    for root in order.into_iter().rev() {
        if labels[root].is_some() {
            continue;
        }
        labels[root] = Some(root);
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            for &parent in &reverse[i] {
                if labels[parent].is_none() {
                    labels[parent] = Some(root);
                    stack.push(parent)
                }
            }
        }
    }
    labels.into_iter().map(Option::unwrap).collect()
}
pub(super) fn closure(lock: &LibraryLock, s: &mut Shape) {
    unique(
        lock.registries
            .iter()
            .enumerate()
            .map(|(i, r)| (format!("/registries/{i}/id"), r.id.as_str())),
        s,
    );
    let mut seen = HashSet::new();
    let mut first = Vec::new();
    for (i, a) in lock.acquisitions.iter().enumerate() {
        if seen.insert(&a.release) {
            first.push((i, a))
        } else {
            s.add(
                format!("/acquisitions/{i}/release"),
                Rule::DuplicateIdentity,
            )
        }
    }
    let mut paths = HashSet::new();
    for (i, a) in &first {
        if !paths.insert((&a.registry, a.record.path())) {
            s.add(
                format!("/acquisitions/{i}/record/path"),
                Rule::DuplicateIdentity,
            )
        }
    }
    unique(
        lock.evidence
            .iter()
            .enumerate()
            .map(|(i, e)| (format!("/evidence/{i}/id"), e.id.as_str())),
        s,
    );
    let mut paths = HashSet::new();
    for (i, e) in lock.evidence.iter().enumerate() {
        if !paths.insert((&e.registry, e.reference.path())) {
            s.add(format!("/evidence/{i}/path"), Rule::DuplicateIdentity)
        }
    }
    let registries: HashSet<_> = lock.registries.iter().map(|r| &r.id).collect();
    let evidence: HashMap<_, _> = lock.evidence.iter().map(|e| (&e.id, e)).collect();
    let mut counts: HashMap<&LocalId, usize> = HashMap::new();
    let mut roles: HashMap<(&LocalId, EvidenceKind), Vec<(usize, &LibraryEvidence)>> =
        HashMap::new();
    for (i, e) in lock.evidence.iter().enumerate() {
        *counts.entry(&e.id).or_default() += 1;
        roles.entry((&e.registry, e.kind)).or_default().push((i, e));
    }
    let acquired: HashSet<_> = lock.acquisitions.iter().map(|a| &a.registry).collect();
    let statements: HashSet<_> = lock.acquisitions.iter().map(|a| &a.statement).collect();
    let acquisitions: HashSet<_> = lock.acquisitions.iter().map(|a| &a.release).collect();
    let nodes: HashSet<_> = lock.graph.nodes().iter().map(|n| n.release()).collect();
    let complete_acquisitions = nodes == acquisitions;
    let complete_statements = first.iter().all(|(_, a)| {
        evidence
            .get(&a.statement)
            .is_some_and(|e| e.kind == EvidenceKind::ReleaseStatement && e.registry == a.registry)
    });
    for (i, n) in lock.graph.nodes().iter().enumerate() {
        if !acquisitions.contains(n.release()) {
            s.add(format!("/graph/nodes/{i}/release"), Rule::MissingReference)
        }
    }
    let reference =
        |id: &LocalId, registry: &LocalId, kind: EvidenceKind, p: String, s: &mut Shape| {
            if counts.get(id).is_some_and(|n| *n > 1) {
                return;
            }
            match evidence.get(id) {
                None => s.add(p, Rule::MissingReference),
                Some(e) if e.kind != kind => s.add(p, Rule::EvidenceKindMismatch),
                Some(e) if &e.registry != registry => s.add(p, Rule::IdentityMismatch),
                _ => {}
            }
        };
    for (i, a) in first {
        if !nodes.contains(&a.release) {
            s.add(format!("/acquisitions/{i}/release"), Rule::OrphanReference)
        }
        if !registries.contains(&a.registry) {
            s.add(
                format!("/acquisitions/{i}/registry"),
                Rule::MissingReference,
            )
        }
        reference(
            &a.statement,
            &a.registry,
            EvidenceKind::ReleaseStatement,
            format!("/acquisitions/{i}/statement"),
            s,
        );
    }
    for (i, r) in lock.registries.iter().enumerate() {
        if !acquired.contains(&r.id) {
            s.add(format!("/registries/{i}/id"), Rule::OrphanReference)
        }
        reference(
            &r.snapshot,
            &r.id,
            EvidenceKind::TufSnapshot,
            format!("/registries/{i}/snapshot"),
            s,
        );
        for kind in [
            EvidenceKind::TufRoot,
            EvidenceKind::TufTimestamp,
            EvidenceKind::TufSnapshot,
            EvidenceKind::TufTargets,
        ] {
            let matches = roles
                .get(&(&r.id, kind))
                .map(Vec::as_slice)
                .unwrap_or_default();
            if matches.is_empty() {
                s.add(format!("/registries/{i}"), Rule::MissingReference)
            }
            if kind != EvidenceKind::TufRoot {
                for (index, _) in matches.iter().skip(1) {
                    s.add(format!("/evidence/{index}"), Rule::OrphanReference)
                }
            }
        }
    }
    for (i, e) in lock.evidence.iter().enumerate() {
        if !registries.contains(&e.registry) {
            s.add(format!("/evidence/{i}/registry"), Rule::MissingReference)
        }
        if complete_acquisitions
            && complete_statements
            && e.kind == EvidenceKind::ReleaseStatement
            && !statements.contains(&e.id)
        {
            s.add(format!("/evidence/{i}/id"), Rule::OrphanReference)
        }
    }
}
