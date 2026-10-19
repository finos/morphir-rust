# morphir-json

A serde-only literal JSON value traversal shared by Morphir's bounded source
reader and extension SDK. Keys that resemble serde internals remain ordinary
keys. Ordinary duplicates use last-wins replacement. Active serde_json numeric
precision and map ordering remain in effect.

The caller supplies a `ReadBudget`, bounds input bytes, and refuses excessive
value depth/count before materialization. Keys and decoded string values have a
before-owned-copy callback. Escaped-string scratch belongs to serde_json and is
bounded by the caller's source byte limit. This is not an exact heap/RSS bound.
See the crate documentation for a finite-budget example.

This crate does not define strict RPC duplicate/closed-shape policies, host
retention authority, typed Morphir IR semantics, or YAML/Ion parsing. The
extension SDK keeps its existing RawValue validation and opaque-field nesting
policy when using this traversal.
