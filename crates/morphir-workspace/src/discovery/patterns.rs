//! Confined workspace member and exclusion patterns.

use std::collections::BTreeSet;

use globset::{GlobBuilder, GlobMatcher};

use crate::{
    DiscoveryFailure, FileTree, RelativePath, WORKSPACE_MEMBER_INVALID, WORKSPACE_PATH_NOT_CONFINED,
};

use super::diagnostics::failure;

pub(super) fn member_directories(
    tree: &FileTree,
    config_anchor: &RelativePath,
    members: &[String],
    excludes: &[String],
) -> Result<Vec<RelativePath>, DiscoveryFailure> {
    members
        .iter()
        .chain(excludes)
        .try_for_each(|pattern| validate_pattern(pattern, config_anchor))?;
    let member_matchers = compile_patterns(members, config_anchor)?;
    let exclude_matchers = compile_patterns(excludes, config_anchor)?;
    let matches = tree
        .directories()
        .filter(|directory| {
            member_matchers
                .iter()
                .any(|matcher| matcher.is_match(directory.as_str()))
        })
        .filter(|directory| {
            !exclude_matchers
                .iter()
                .any(|matcher| matcher.is_match(directory.as_str()))
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(matches.into_iter().collect())
}

fn compile_patterns(
    patterns: &[String],
    config_anchor: &RelativePath,
) -> Result<Vec<GlobMatcher>, DiscoveryFailure> {
    patterns
        .iter()
        .map(|pattern| {
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map(|glob| glob.compile_matcher())
                .map_err(|error| {
                    failure(
                        WORKSPACE_MEMBER_INVALID,
                        format!("invalid workspace member pattern `{pattern}`: {error}"),
                        Some(config_anchor.clone()),
                    )
                })
        })
        .collect()
}

fn validate_pattern(pattern: &str, config_anchor: &RelativePath) -> Result<(), DiscoveryFailure> {
    let bytes = pattern.as_bytes();
    let windows_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if pattern.starts_with('/')
        || pattern.contains('\\')
        || windows_prefix
        || pattern.split('/').any(|component| component == "..")
    {
        return Err(failure(
            WORKSPACE_PATH_NOT_CONFINED,
            format!("workspace pattern `{pattern}` is not confined to the development root"),
            Some(config_anchor.clone()),
        ));
    }
    Ok(())
}
