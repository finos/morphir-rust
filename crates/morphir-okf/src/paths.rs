//! Segment arithmetic over paths, ported from `KbPath` in `KbModel.scala`.
//!
//! Containment and relativization are component operations on `std::path`,
//! which compares by components rather than by string.

use std::path::{Component, Path};

/// True when `child` sits at or below `base`.
pub fn is_under(child: &Path, base: &Path) -> bool {
    child.starts_with(base)
}

/// Segments of `child` below `base`, or `None` when `child` is not under
/// `base`.
pub fn segments_under(child: &Path, base: &Path) -> Option<Vec<String>> {
    child.strip_prefix(base).ok().map(|rest| {
        rest.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect()
    })
}

/// Renders a path with forward slashes and a leading `/`, dropping root and
/// prefix components — the display form used in findings and messages.
pub fn render(p: &Path) -> String {
    let segs: Vec<_> = p
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    format!("/{}", segs.join("/"))
}
