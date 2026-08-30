//! Bounded materialization of confined directory aliases.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow, bail};
use morphir_workspace::{FileEntry, RelativePath};

#[derive(Debug)]
pub(super) struct DirectoryAlias {
    pub(super) lexical_path: RelativePath,
    pub(super) canonical_target: RelativePath,
}

pub(super) fn record_directory_alias(
    aliases: &mut Vec<DirectoryAlias>,
    budgets: AliasBudgets,
    make_alias: impl FnOnce() -> DirectoryAlias,
) -> Result<()> {
    if aliases.len() >= budgets.alias_edges {
        return alias_resource_limit("alias edges", budgets.alias_edges);
    }
    aliases.push(make_alias());
    Ok(())
}

pub(super) fn materialize_directory_aliases(
    aliases: &mut [DirectoryAlias],
    entries: &mut BTreeMap<RelativePath, FileEntry>,
    budgets: AliasBudgets,
) -> Result<()> {
    aliases.sort_by(|left, right| left.lexical_path.cmp(&right.lexical_path));
    if aliases.len() > budgets.alias_edges {
        return alias_resource_limit("alias edges", budgets.alias_edges);
    }
    if aliases.is_empty() {
        return Ok(());
    }
    let mut stats = AliasStats::default();
    let mut real_entries = BTreeMap::new();
    for (path, entry) in entries.iter() {
        stats.work(budgets)?;
        real_entries.insert(path.clone(), entry.clone());
    }
    let mut edges = Vec::with_capacity(aliases.len());
    for alias in aliases {
        stats.work(budgets)?;
        edges.push(ResolvedAlias {
            lexical_path: alias.lexical_path.clone(),
            real_target: alias.canonical_target.clone(),
        });
    }
    let mut edge_index = BTreeMap::new();
    for (edge_id, edge) in edges.iter().enumerate() {
        stats.work(budgets)?;
        edge_index.insert(edge.lexical_path.clone(), edge_id);
    }
    let mut worklist = BTreeSet::new();
    for (edge_id, edge) in edges.iter().enumerate() {
        stats.enqueue(budgets)?;
        worklist.insert(AliasExpansion {
            lexical_path: edge.lexical_path.clone(),
            edge_id,
            ancestry: vec![edge_id],
        });
    }
    let mut indexes = BTreeMap::<RelativePath, AliasIndex>::new();

    while let Some(expansion) = worklist.pop_first() {
        stats.process(budgets)?;
        let edge = &edges[expansion.edge_id];
        if !indexes.contains_key(&edge.real_target) {
            let index = build_alias_index(
                &edge.real_target,
                &real_entries,
                &edge_index,
                budgets,
                &mut stats,
            )?;
            stats.work(budgets)?;
            indexes.insert(edge.real_target.clone(), index);
        }
        let index = indexes
            .get(&edge.real_target)
            .expect("alias target index was just built");
        for (suffix, entry) in &index.entries {
            stats.work(budgets)?;
            let alias_path =
                if suffix.is_empty() {
                    expansion.lexical_path.clone()
                } else {
                    expansion.lexical_path.join(suffix.as_str()).map_err(|error| {
                    anyhow!(
                        "workspace.path.not-confined: cannot materialize alias `{}`: {error}",
                        expansion.lexical_path.as_str()
                    )
                })?
                };
            if let std::collections::btree_map::Entry::Vacant(slot) = entries.entry(alias_path) {
                stats.generate(budgets)?;
                slot.insert(entry.clone());
            }
        }

        for (suffix, nested_id) in &index.nested_aliases {
            if expansion.ancestry.binary_search(nested_id).is_ok() {
                continue;
            }
            stats.work(budgets)?;
            let nested_lexical_path = if suffix.is_empty() {
                expansion.lexical_path.clone()
            } else {
                expansion.lexical_path.join(suffix.as_str()).map_err(|error| {
                    anyhow!(
                        "workspace.path.not-confined: cannot materialize nested alias below `{}`: {error}",
                        expansion.lexical_path.as_str()
                    )
                })?
            };
            let mut ancestry = expansion.ancestry.clone();
            ancestry.push(*nested_id);
            ancestry.sort_unstable();
            let nested = AliasExpansion {
                lexical_path: nested_lexical_path,
                edge_id: *nested_id,
                ancestry,
            };
            if !worklist.contains(&nested) {
                stats.enqueue(budgets)?;
                worklist.insert(nested);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AliasBudgets {
    pub(super) alias_edges: usize,
    pub(super) queued_expansions: usize,
    pub(super) processed_expansions: usize,
    pub(super) generated_entries: usize,
    pub(super) total_work: usize,
}

impl AliasBudgets {
    pub(super) const DEFAULT: Self = Self {
        alias_edges: 4_096,
        queued_expansions: 32_768,
        processed_expansions: 32_768,
        generated_entries: 262_144,
        total_work: 2_000_000,
    };
}

#[derive(Default)]
struct AliasStats {
    queued_expansions: usize,
    processed_expansions: usize,
    generated_entries: usize,
    total_work: usize,
}

impl AliasStats {
    fn enqueue(&mut self, budgets: AliasBudgets) -> Result<()> {
        increment_bounded(
            &mut self.queued_expansions,
            budgets.queued_expansions,
            "queued expansions",
        )?;
        self.work(budgets)
    }

    fn process(&mut self, budgets: AliasBudgets) -> Result<()> {
        increment_bounded(
            &mut self.processed_expansions,
            budgets.processed_expansions,
            "processed expansions",
        )?;
        self.work(budgets)
    }

    fn generate(&mut self, budgets: AliasBudgets) -> Result<()> {
        increment_bounded(
            &mut self.generated_entries,
            budgets.generated_entries,
            "generated entries",
        )?;
        self.work(budgets)
    }

    fn work(&mut self, budgets: AliasBudgets) -> Result<()> {
        increment_bounded(&mut self.total_work, budgets.total_work, "total work")
    }
}

fn increment_bounded(value: &mut usize, limit: usize, resource: &str) -> Result<()> {
    if *value >= limit {
        return alias_resource_limit(resource, limit);
    }
    *value += 1;
    Ok(())
}

fn alias_resource_limit<T>(resource: &str, limit: usize) -> Result<T> {
    bail!(
        "workspace.alias.resource-limit: confined alias graph exceeded fixed {resource} budget {limit}"
    )
}

struct AliasIndex {
    entries: Vec<(String, FileEntry)>,
    nested_aliases: Vec<(String, usize)>,
}

fn build_alias_index(
    target: &RelativePath,
    real_entries: &BTreeMap<RelativePath, FileEntry>,
    edge_index: &BTreeMap<RelativePath, usize>,
    budgets: AliasBudgets,
    stats: &mut AliasStats,
) -> Result<AliasIndex> {
    let entries = index_real_subtree(target, real_entries, budgets, stats)?;
    let nested_aliases = index_alias_subtree(target, edge_index, budgets, stats)?;
    Ok(AliasIndex {
        entries,
        nested_aliases,
    })
}

fn index_alias_subtree(
    target: &RelativePath,
    edge_index: &BTreeMap<RelativePath, usize>,
    budgets: AliasBudgets,
    stats: &mut AliasStats,
) -> Result<Vec<(String, usize)>> {
    if target == &RelativePath::root() {
        return edge_index
            .iter()
            .map(|(path, edge_id)| {
                stats.work(budgets)?;
                Ok((
                    relative_suffix(path, target)
                        .expect("every confined alias path is below the root")
                        .to_owned(),
                    *edge_id,
                ))
            })
            .collect();
    }

    let mut indexed = Vec::new();
    stats.work(budgets)?;
    if let Some(edge_id) = edge_index.get(target) {
        indexed.push((String::new(), *edge_id));
    }
    stats.work(budgets)?;
    let prefix = format!("{}/", target.as_str());
    let upper = RelativePath::parse(format!("{}0", target.as_str()))
        .expect("appending a confined component character remains confined");
    for (path, edge_id) in edge_index.range((
        std::ops::Bound::Excluded(target.clone()),
        std::ops::Bound::Excluded(upper),
    )) {
        stats.work(budgets)?;
        if !path.as_str().starts_with(&prefix) {
            continue;
        }
        indexed.push((path.as_str()[prefix.len()..].to_owned(), *edge_id));
    }
    Ok(indexed)
}

fn index_real_subtree(
    target: &RelativePath,
    real_entries: &BTreeMap<RelativePath, FileEntry>,
    budgets: AliasBudgets,
    stats: &mut AliasStats,
) -> Result<Vec<(String, FileEntry)>> {
    if target == &RelativePath::root() {
        return real_entries
            .iter()
            .map(|(path, entry)| {
                stats.work(budgets)?;
                Ok((
                    relative_suffix(path, target)
                        .expect("every confined path is below the root")
                        .to_owned(),
                    entry.clone(),
                ))
            })
            .collect();
    }

    let mut indexed = Vec::new();
    stats.work(budgets)?;
    if let Some(entry) = real_entries.get(target) {
        stats.work(budgets)?;
        indexed.push((String::new(), entry.clone()));
    }
    stats.work(budgets)?;
    let prefix = format!("{}/", target.as_str());
    let upper = RelativePath::parse(format!("{}0", target.as_str()))
        .expect("appending a confined component character remains confined");
    for (path, entry) in real_entries.range((
        std::ops::Bound::Excluded(target.clone()),
        std::ops::Bound::Excluded(upper),
    )) {
        stats.work(budgets)?;
        if !path.as_str().starts_with(&prefix) {
            continue;
        }
        stats.work(budgets)?;
        indexed.push((path.as_str()[prefix.len()..].to_owned(), entry.clone()));
    }
    Ok(indexed)
}

struct ResolvedAlias {
    lexical_path: RelativePath,
    real_target: RelativePath,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AliasExpansion {
    lexical_path: RelativePath,
    edge_id: usize,
    ancestry: Vec<usize>,
}

fn relative_suffix<'a>(path: &'a RelativePath, root: &RelativePath) -> Option<&'a str> {
    if root.as_str() == "." {
        return Some(if path.as_str() == "." {
            ""
        } else {
            path.as_str()
        });
    }
    if path == root {
        return Some("");
    }
    path.as_str().strip_prefix(root.as_str())?.strip_prefix('/')
}
