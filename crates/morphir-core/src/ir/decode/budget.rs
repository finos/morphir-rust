//! Validated policy and aggregate, non-resetting per-attempt ledgers.
use morphir_json::{ReadBudget, ValueKind};
use std::{error::Error, fmt};

const BYTE_LIMIT: usize = 64 * 1024 * 1024;
const DEPTH_LIMIT: usize = 128;
const VALUE_LIMIT: usize = 100_000;

/// Which independently bounded representation is being built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodePhase {
    /// Physical source values, aggregated across a selected primary tree.
    Source,
    /// The assembled semantic representation, independently bounded.
    Assembled,
}
/// The resource that refused further work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeLimit {
    /// Aggregate source bytes.
    SourceBytes,
    /// Root-zero value depth.
    Depth,
    /// Scalars and containers; object keys are not values.
    Values,
    /// Decoded UTF-16 units in keys and string values.
    StringUnits,
}
/// A bounded, typed refusal. No source text is retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimitExceeded {
    phase: DecodePhase,
    limit: DecodeLimit,
}
impl DecodeLimitExceeded {
    /// Representation that was being built.
    pub fn phase(self) -> DecodePhase {
        self.phase
    }
    /// Refused resource.
    pub fn limit(self) -> DecodeLimit {
        self.limit
    }
}
impl fmt::Display for DecodeLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} {:?} decode limit exceeded", self.phase, self.limit)
    }
}
impl Error for DecodeLimitExceeded {}

/// Limits outside the additive model-originals policy ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidDecodeLimits;
impl fmt::Display for InvalidDecodeLimits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid model-originals decode limits")
    }
}
impl Error for InvalidDecodeLimits {}

/// Finite policy, at or below 64 MiB / depth 128 / 100,000 values per phase.
///
/// Root depth is zero. Each scalar and container consumes one value, including
/// values later replaced by a duplicate object member. Keys do not consume
/// values. Keys and strings together consume UTF-16 units. Each source and
/// assembled phase has its own inclusive unit/value limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    bytes: usize,
    depth: usize,
    values: usize,
    string_units: usize,
}
impl DecodeLimits {
    /// The inclusive model-originals ceiling. This is not a heap/RSS guarantee.
    pub const fn model_originals() -> Self {
        Self {
            bytes: BYTE_LIMIT,
            depth: DEPTH_LIMIT,
            values: VALUE_LIMIT,
            string_units: BYTE_LIMIT,
        }
    }
    /// Select stricter limits. Bytes/values must be positive; depth and string
    /// units may be zero. No argument may exceed the model-originals ceiling.
    pub fn new(
        bytes: usize,
        depth: usize,
        values: usize,
        string_units: usize,
    ) -> Result<Self, InvalidDecodeLimits> {
        if bytes == 0
            || bytes > BYTE_LIMIT
            || depth > DEPTH_LIMIT
            || values == 0
            || values > VALUE_LIMIT
            || string_units > BYTE_LIMIT
        {
            return Err(InvalidDecodeLimits);
        }
        Ok(Self {
            bytes,
            depth,
            values,
            string_units,
        })
    }
}
/// Observed resource use. Failed attempts consume reservations already made.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DecodeUsage {
    values: usize,
    string_units: usize,
}
impl DecodeUsage {
    /// Source values visited, or assembled values checked.
    pub fn values(self) -> usize {
        self.values
    }
    /// Decoded UTF-16 units reserved before owned string copies.
    pub fn string_units(self) -> usize {
        self.string_units
    }
}
/// Mutable ownership of one checked decode attempt's aggregate budgets.
///
/// Source parsing and assembled validation have separate ledgers. Header facts
/// derived from the parsed value do not reparse or charge another model. A
/// separate source document shares the source ledger. Start a new attempt with
/// a new budget; no reset or unaccounted fallback operation is provided.
#[derive(Debug)]
pub struct DecodeBudget {
    source: PhaseBudget,
    assembled: PhaseBudget,
    source_bytes: usize,
}
impl DecodeBudget {
    /// Start a new attempt with a validated finite policy.
    pub fn new(limits: DecodeLimits) -> Self {
        Self {
            source: PhaseBudget::new(limits, DecodePhase::Source),
            assembled: PhaseBudget::new(limits, DecodePhase::Assembled),
            source_bytes: 0,
        }
    }
    /// Read-only phase usage for diagnostics and pre-growth tests.
    pub fn usage(&self, phase: DecodePhase) -> DecodeUsage {
        match phase {
            DecodePhase::Source => self.source.usage,
            DecodePhase::Assembled => self.assembled.usage,
        }
    }
    /// Aggregate primary-source bytes reserved before parsing.
    pub fn source_bytes(&self) -> usize {
        self.source_bytes
    }
    pub(super) fn reserve_source(&mut self, bytes: usize) -> Result<(), DecodeLimitExceeded> {
        self.source_bytes = self
            .source_bytes
            .checked_add(bytes)
            .filter(|n| *n <= self.source.limits.bytes)
            .ok_or(DecodeLimitExceeded {
                phase: DecodePhase::Source,
                limit: DecodeLimit::SourceBytes,
            })?;
        Ok(())
    }
    pub(crate) fn source(&mut self) -> &mut PhaseBudget {
        &mut self.source
    }
    pub(super) fn assembled(&mut self) -> &mut PhaseBudget {
        &mut self.assembled
    }
}
#[derive(Debug)]
pub(crate) struct PhaseBudget {
    limits: DecodeLimits,
    phase: DecodePhase,
    usage: DecodeUsage,
}
impl PhaseBudget {
    pub(crate) fn depth_limit(&self) -> usize {
        self.limits.depth
    }
    pub(crate) fn depth_refusal(&self) -> DecodeLimitExceeded {
        self.exceeded(DecodeLimit::Depth)
    }
    fn new(limits: DecodeLimits, phase: DecodePhase) -> Self {
        Self {
            limits,
            phase,
            usage: DecodeUsage::default(),
        }
    }
    fn exceeded(&self, limit: DecodeLimit) -> DecodeLimitExceeded {
        DecodeLimitExceeded {
            phase: self.phase,
            limit,
        }
    }
}
impl ReadBudget for PhaseBudget {
    type Error = DecodeLimitExceeded;
    fn before_value(&mut self, depth: usize, _: ValueKind) -> Result<(), Self::Error> {
        if depth > self.limits.depth {
            return Err(self.exceeded(DecodeLimit::Depth));
        }
        if self.usage.values >= self.limits.values {
            return Err(self.exceeded(DecodeLimit::Values));
        }
        self.usage.values += 1;
        Ok(())
    }
    fn before_string(&mut self, value: &str) -> Result<(), Self::Error> {
        // Count without allocating a UTF-16 copy, stopping at the first excess.
        let remaining = self.limits.string_units - self.usage.string_units;
        let units = value.encode_utf16().take(remaining + 1).count();
        if units > remaining {
            return Err(self.exceeded(DecodeLimit::StringUnits));
        }
        self.usage.string_units += units;
        Ok(())
    }
}
