//! Support tables: unions of intervals over releases, in Maven-style interval
//! notation with one canonical spelling. docs/spec/ir/format-version.md,
//! "Recognition and compatibility".

use super::diagnostic::FormatVersionDiagnostic;
use super::triplet::ReleaseTriplet;
use std::cmp::Ordering;

/// The reference table of the parent specification.
pub const REFERENCE_SUPPORT_TABLE: &str = "[3.0.0,3.1.0),[4.0.0,4.1.0)";

/// The smallest release the domain has.
///
/// [`parse_release`] refuses a bound below major 3, so an absent lower bound
/// admits from here and not from `0.0.0`: a table is a set of releases the
/// contract can name, and it names none earlier.
const DOMAIN_FLOOR: ReleaseTriplet = ReleaseTriplet::new(3, 0, 0);

/// Compatibility result after normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    /// The normalized release lies inside the table.
    Supported,
    /// No interval of the table contains any release of the release's major family.
    UnsupportedMajor,
    /// Some interval contains a release of that major family, but none contains this release.
    UnsupportedMinor,
}

/// One interval of a support table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interval {
    /// Lower bound, or `None` for no lower bound.
    pub lower: Option<ReleaseTriplet>,
    /// Whether the lower bound itself is admitted.
    pub lower_inclusive: bool,
    /// Upper bound, or `None` for no upper bound.
    pub upper: Option<ReleaseTriplet>,
    /// Whether the upper bound itself is admitted.
    pub upper_inclusive: bool,
}

/// A union of intervals over releases, held in canonical (normalised) form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportTable {
    intervals: Vec<Interval>,
}

fn parse_release(text: &str, r#where: &str) -> Result<ReleaseTriplet, String> {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() != 3 {
        return Err(format!("{where}: \"{text}\" is not a release string"));
    }
    let mut out = [0u32; 3];
    for (i, part) in parts.iter().enumerate() {
        let valid = !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (*part == "0" || !part.starts_with('0'));
        if !valid {
            return Err(format!("{where}: \"{text}\" is not a release string"));
        }
        out[i] = part
            .parse::<u32>()
            .map_err(|_| format!("{where}: \"{text}\" has a component above {}", u32::MAX))?;
    }
    if out[0] < 3 {
        return Err(format!(
            "{where}: release strings are valid only for major 3 and later, got \"{text}\""
        ));
    }
    Ok(ReleaseTriplet::new(out[0], out[1], out[2]))
}

impl Interval {
    /// The smallest release this interval's lower bound admits, or `None` when
    /// the bound admits no release at all.
    fn smallest_admitted(&self) -> Option<ReleaseTriplet> {
        match self.lower {
            None => Some(DOMAIN_FLOOR),
            Some(lower) if self.lower_inclusive => Some(lower),
            Some(lower) => lower.successor(),
        }
    }

    /// Does `release` satisfy the upper bound? A missing upper bound admits everything.
    fn below_upper(&self, release: &ReleaseTriplet) -> bool {
        match &self.upper {
            None => true,
            Some(upper) => match release.cmp(upper) {
                Ordering::Less => true,
                Ordering::Equal => self.upper_inclusive,
                Ordering::Greater => false,
            },
        }
    }

    fn normalised(mut self, r#where: &str) -> Result<Self, String> {
        if let (Some(lower), false) = (self.lower, self.lower_inclusive)
            && let Some(next) = lower.next()
        {
            self.lower = Some(next);
            self.lower_inclusive = true;
        }
        if let (Some(upper), true) = (self.upper, self.upper_inclusive)
            && let Some(next) = upper.next()
        {
            self.upper = Some(next);
            self.upper_inclusive = false;
        }
        if self.lower.is_none() {
            self.lower_inclusive = false;
        }
        if self.upper.is_none() {
            self.upper_inclusive = false;
        }
        // Empty: the smallest release the lower bound admits either does not
        // exist at all, or does not satisfy the upper bound.
        match self.smallest_admitted() {
            Some(smallest) if self.below_upper(&smallest) => Ok(self),
            _ => Err(format!("{where}: the interval contains no release")),
        }
    }

    fn contains(&self, r: &ReleaseTriplet) -> bool {
        // An absent lower bound reaches down to the domain floor and no
        // further, so the floor is checked here rather than only where a bound
        // is written: a table cannot contain a release the domain does not have.
        if *r < DOMAIN_FLOOR {
            return false;
        }
        if let Some(lower) = &self.lower {
            match lower.cmp(r) {
                Ordering::Greater => return false,
                Ordering::Equal if !self.lower_inclusive => return false,
                _ => {}
            }
        }
        self.below_upper(r)
    }

    /// Does this interval contain at least one release of `major`?
    ///
    /// The question is containment, not the majors the bounds happen to be
    /// spelled with: `[3.0.0,4.0.0)` holds no release of major 4, while
    /// `[3.0.0,4.0.1)` holds `4.0.0`.
    fn contains_major(&self, major: u32) -> bool {
        let family_start = ReleaseTriplet::new(major, 0, 0);
        let Some(admitted) = self.smallest_admitted() else {
            return false;
        };
        let candidate = if admitted < family_start {
            family_start
        } else {
            admitted
        };
        candidate.major() == major && self.below_upper(&candidate)
    }

    /// Does this interval's upper reach `next`'s lower (overlap or adjacency)?
    /// Both are normalised.
    ///
    /// Adjacency is a question about the release set, not about the numerals,
    /// so it asks [`ReleaseTriplet::successor`] the way emptiness does: nothing
    /// lies between `4.0.4294967295` and `4.1.0`, so `[4.0.0,4.0.4294967295]`
    /// and `[4.1.0,4.2.0)` are adjacent and merge. Without that, one release
    /// set would have two canonical spellings.
    fn touches(&self, next: &Interval) -> bool {
        match (&self.upper, &next.lower) {
            (None, _) | (_, None) => true,
            (Some(upper), Some(lower)) => match lower.cmp(upper) {
                Ordering::Less => true,
                // [x,y) then [y,..) is adjacent; [x,y] then [y,..) overlaps;
                // (y,..) after ..,y] is adjacent.
                Ordering::Equal => self.upper_inclusive || next.lower_inclusive,
                // Above the upper bound: an exclusive upper already stops short
                // of its own bound, so only an inclusive one can still be
                // adjacent, and only to its successor.
                Ordering::Greater => {
                    self.upper_inclusive && upper.successor().is_some_and(|after| *lower <= after)
                }
            },
        }
    }

    fn canonical(&self) -> String {
        let open = if self.lower.is_some() && self.lower_inclusive {
            '['
        } else {
            '('
        };
        let close = if self.upper.is_some() && self.upper_inclusive {
            ']'
        } else {
            ')'
        };
        let lo = self.lower.map(|r| r.to_exact_string()).unwrap_or_default();
        let hi = self.upper.map(|r| r.to_exact_string()).unwrap_or_default();
        format!("{open}{lo},{hi}{close}")
    }
}

impl SupportTable {
    /// Parse a support table from the interval notation, normalising it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut intervals = Vec::new();
        let bytes = text.as_bytes();
        let mut pos = 0;
        let mut first = true;
        loop {
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if pos >= bytes.len() {
                break;
            }
            if !first {
                if bytes[pos] != b',' {
                    return Err(format!("expected \",\" at offset {pos} in \"{text}\""));
                }
                pos += 1;
                while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                    pos += 1;
                }
            }
            let open = match bytes.get(pos) {
                Some(b'[') => true,
                Some(b'(') => false,
                _ => {
                    return Err(format!(
                        "expected an interval at offset {pos} in \"{text}\""
                    ));
                }
            };
            let end = text[pos..]
                .find([']', ')'])
                .map(|i| pos + i)
                .ok_or_else(|| format!("unterminated interval at offset {pos} in \"{text}\""))?;
            let close = bytes[end] == b']';
            let inner = &text[pos + 1..end];
            if inner.contains(['[', '(']) {
                return Err(format!("expected \",\" between intervals in \"{text}\""));
            }
            let r#where: String = text[pos..=end]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            let interval = match parts.as_slice() {
                [exact] => {
                    if !(open && close) {
                        return Err(format!("{where}: an exact release uses square brackets"));
                    }
                    let r = parse_release(exact, &r#where)?;
                    Interval {
                        lower: Some(r),
                        lower_inclusive: true,
                        upper: Some(r),
                        upper_inclusive: true,
                    }
                }
                [lo, hi] => {
                    if lo.is_empty() && hi.is_empty() {
                        return Err(format!("{where}: at least one bound is required"));
                    }
                    let lower = if lo.is_empty() {
                        None
                    } else {
                        Some(parse_release(lo, &r#where)?)
                    };
                    let upper = if hi.is_empty() {
                        None
                    } else {
                        Some(parse_release(hi, &r#where)?)
                    };
                    Interval {
                        lower,
                        lower_inclusive: open,
                        upper,
                        upper_inclusive: close,
                    }
                }
                _ => return Err(format!("{where}: an interval has at most two bounds")),
            };
            intervals.push(interval.normalised(&r#where)?);
            pos = end + 1;
            first = false;
        }
        if intervals.is_empty() {
            return Err("a support table needs at least one interval".to_owned());
        }
        let merged = Self::merged(intervals);
        // Each interval is bounded on at least one side, but two of them can
        // cover each other's open side and merge into the interval with no
        // bounds at all. That has no spelling the grammar accepts, and a table
        // that excludes no release is not a support claim, so it is rejected
        // here rather than written.
        if merged
            .intervals
            .iter()
            .any(|i| i.lower.is_none() && i.upper.is_none())
        {
            return Err(format!(
                "\"{text}\": the table admits every release; a support table must exclude some release"
            ));
        }
        Ok(merged)
    }

    fn merged(mut intervals: Vec<Interval>) -> Self {
        intervals.sort_by(|a, b| match (&a.lower, &b.lower) {
            (None, None) => Ordering::Equal,
            (None, _) => Ordering::Less,
            (_, None) => Ordering::Greater,
            (Some(x), Some(y)) => x.cmp(y),
        });
        let mut out: Vec<Interval> = Vec::new();
        for i in intervals {
            match out.last_mut() {
                Some(last) if last.touches(&i) => {
                    let (upper, upper_inclusive) = match (&last.upper, &i.upper) {
                        (None, _) | (_, None) => (None, false),
                        (Some(a), Some(b)) => match a.cmp(b) {
                            Ordering::Greater => (Some(*a), last.upper_inclusive),
                            Ordering::Less => (Some(*b), i.upper_inclusive),
                            Ordering::Equal => {
                                (Some(*a), last.upper_inclusive || i.upper_inclusive)
                            }
                        },
                    };
                    last.upper = upper;
                    last.upper_inclusive = upper_inclusive;
                }
                _ => out.push(i),
            }
        }
        Self { intervals: out }
    }

    /// Each release as an exact interval, normalised and merged.
    pub fn from_releases(releases: impl IntoIterator<Item = ReleaseTriplet>) -> Self {
        Self::merged(
            releases
                .into_iter()
                .map(|r| {
                    Interval {
                        lower: Some(r),
                        lower_inclusive: true,
                        upper: Some(r),
                        upper_inclusive: true,
                    }
                    .normalised(&r.to_exact_string())
                    .expect("an exact interval is never empty")
                })
                .collect(),
        )
    }

    /// Return the reference support table from the parent specification.
    pub fn reference() -> Self {
        Self::parse(REFERENCE_SUPPORT_TABLE).expect("the reference table parses")
    }

    /// The table's intervals, in canonical order.
    pub fn intervals(&self) -> &[Interval] {
        &self.intervals
    }

    /// The table's single canonical spelling.
    pub fn canonical(&self) -> String {
        self.intervals
            .iter()
            .map(Interval::canonical)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Return `true` when the release lies inside any interval of the table.
    pub fn contains(&self, release: &ReleaseTriplet) -> bool {
        self.intervals.iter().any(|i| i.contains(release))
    }

    /// Check compatibility for one normalized release.
    pub fn check(&self, release: &ReleaseTriplet) -> Compatibility {
        if self.contains(release) {
            Compatibility::Supported
        } else if self
            .intervals
            .iter()
            .any(|i| i.contains_major(release.major()))
        {
            Compatibility::UnsupportedMinor
        } else {
            Compatibility::UnsupportedMajor
        }
    }

    /// Convert a compatibility result into a stable diagnostic when unsupported.
    pub fn unsupported_diagnostic(
        &self,
        release: &ReleaseTriplet,
        compatibility: Compatibility,
    ) -> Option<FormatVersionDiagnostic> {
        match compatibility {
            Compatibility::Supported => None,
            Compatibility::UnsupportedMajor => {
                Some(FormatVersionDiagnostic::unsupported_format_version_major(
                    &release.to_exact_string(),
                ))
            }
            Compatibility::UnsupportedMinor => {
                Some(FormatVersionDiagnostic::unsupported_format_version_minor(
                    &release.to_exact_string(),
                ))
            }
        }
    }

    /// Render the table as Cargo comparator sets, one per interval.
    pub fn render_cargo(&self) -> Vec<String> {
        self.intervals
            .iter()
            .map(|i| {
                // Defensive: `parse` rejects a bounds-free interval, so this is
                // unreachable from parsed input. An empty comparator set would
                // read as "any version" by accident, so say it on purpose or
                // not at all.
                if i.lower.is_none() && i.upper.is_none() {
                    return "*".to_owned();
                }
                let mut parts = Vec::new();
                if let Some(lo) = &i.lower {
                    parts.push(format!(
                        "{}{}",
                        if i.lower_inclusive { ">=" } else { ">" },
                        lo.to_exact_string()
                    ));
                }
                if let Some(hi) = &i.upper {
                    parts.push(format!(
                        "{}{}",
                        if i.upper_inclusive { "<=" } else { "<" },
                        hi.to_exact_string()
                    ));
                }
                parts.join(", ")
            })
            .collect()
    }

    /// Render the table as Elm constraints, one per interval.
    pub fn render_elm(&self) -> Result<Vec<String>, String> {
        self.intervals
            .iter()
            .map(|i| match (&i.lower, &i.upper) {
                (Some(lo), Some(hi)) if i.lower_inclusive && !i.upper_inclusive => Ok(format!(
                    "{} <= v < {}",
                    lo.to_exact_string(),
                    hi.to_exact_string()
                )),
                (Some(_), Some(_)) => Err(format!(
                    "Elm constraints are a <= v < b; {} cannot be advanced to that shape",
                    i.canonical()
                )),
                _ => Err(format!(
                    "Elm constraints need both bounds; {} has none on one side",
                    i.canonical()
                )),
            })
            .collect()
    }

    /// Render the table as prose.
    pub fn render_prose(&self) -> String {
        self.intervals
            .iter()
            .map(|i| match (&i.lower, &i.upper) {
                (None, Some(hi)) if i.upper_inclusive => {
                    format!("{} and earlier", hi.to_exact_string())
                }
                (None, Some(hi)) => format!("earlier than {}", hi.to_exact_string()),
                (Some(lo), None) => format!("{} and later", lo.to_exact_string()),
                (Some(lo), Some(hi)) => {
                    let start = if i.lower_inclusive {
                        lo.to_exact_string()
                    } else {
                        format!("after {}", lo.to_exact_string())
                    };
                    if i.upper_inclusive {
                        format!("{start} through {}", hi.to_exact_string())
                    } else {
                        format!("{start} up to but not including {}", hi.to_exact_string())
                    }
                }
                // Defensive, as in `render_cargo`: unreachable from parsed
                // input, and better said plainly than rendered as a bound that
                // is not there.
                (None, None) => "every release".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(", or ")
    }
}

/// Reference support table from the parent specification.
pub fn default_support_table() -> SupportTable {
    SupportTable::reference()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_lower_bound_reaches_only_to_the_domain_floor() {
        let table = SupportTable::parse("(,4.1.0)").expect("an open-below table");
        // The domain names no release before 3.0.0, so a table cannot contain
        // one however far down its lower side is left open.
        for release in [ReleaseTriplet::new(2, 0, 0), ReleaseTriplet::new(1, 0, 0)] {
            assert!(!table.contains(&release), "{release} is below the floor");
            assert_eq!(table.check(&release), Compatibility::UnsupportedMajor);
        }
        assert!(table.contains(&ReleaseTriplet::new(3, 0, 0)));
    }
}
