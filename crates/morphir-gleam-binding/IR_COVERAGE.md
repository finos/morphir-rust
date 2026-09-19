# Gleam IR coverage

This inventory separates tested mappings from remaining semantic work. A parser
accepting source, or a visitor having a match arm, does not establish correct IR.
The source frontend emits values in v4 only. V3 compilation remains type-only;
the backend accepts v3 value IR through migration as well as v4 input.

## Values

| IR value | Current mapping and limits |
| --- | --- |
| Literal | Strings preserve escapes and Unicode. Floats preserve their scalar kind and IEEE bits. Integers and booleans have Gleam spellings. Character, decimal and document literals fail generation because no exact Gleam type mapping is defined. Frontend integers remain limited to signed 64-bit values. |
| Constructor | Local constructor spelling exists. Imported and SDK constructor identity still needs resolution. |
| Tuple | Tuple construction maps to `#(...)`. |
| List | Finite lists map directly. Source lists with a tail lower to SDK `list.cons`; complete applications generate `[head, ..tail]`. |
| Record | Generation rejects anonymous structural records. Named record aliases can generate ADT type declarations; value construction needs nominal type resolution. |
| Variable | Local names map directly. Top-level value references still need distinction from bound variables. |
| Reference | Generic value qualification and SDK function mapping remain incomplete. Complete SDK `list.cons` applications have an explicit mapping. |
| Field | Existing field spelling is retained. ADT field access and tuple indexing need correct IR resolution. |
| FieldFunction | Generation explicitly rejects this variant. |
| Apply | Positional curried IR exists. General Gleam function arity and partial application remain incomplete; SDK `list.cons` has a specific tested mapping. |
| Lambda | Identifier parameters have an existing mapping. General pattern parameters and function arity need further work. |
| LetDefinition | Zero-input local definitions generate expression blocks. Parameterized local definitions fail explicitly pending function-arity support. |
| LetRecursion | Generation explicitly rejects this variant. |
| Destructure | Source blocks preserve bindings, shadowing and preceding expressions. Terminal assignments preserve their pattern and return the RHS once. Generation uses scoped blocks and `let assert` for potentially refutable patterns. |
| IfThenElse | Generation uses a boolean `case` with valid branch syntax. |
| PatternMatch | Single-subject cases map directly, including proper list patterns. Guards and multiple source subjects remain unsupported. |
| UpdateRecord | Generation explicitly rejects this variant pending nominal record resolution. |
| Unit | Backend emits `Nil`. Frontend builtin `Nil` identity still needs correction. |
| Hole | Generation explicitly rejects incomplete values. |

Native, external and incomplete definition bodies fail generation rather than
producing comments or `todo` in a successful artifact. Expression bodies remain
the supported definition form. These explicit failures are diagnostic coverage,
not successful implementation of the rejected constructs.

## Patterns

| IR pattern | Current mapping and limits |
| --- | --- |
| WildcardPattern | Discards map to `_`; preceding source expressions use wildcard destructuring to preserve evaluation. |
| AsPattern | Bindings and aliases retain their names. Generated terminal-assignment aliases avoid collisions with pattern binders. |
| TuplePattern | Tuple patterns map to `#(...)`. |
| ConstructorPattern | Local constructor patterns have a mapping. Qualified constructors, labels and spreads need signature resolution. |
| EmptyListPattern | Empty source patterns and fixed-list endings retain list identity. |
| HeadTailPattern | Nested, fixed-length and open-tail patterns retain every element and tail; generation flattens fixed-list endings into valid Gleam. |
| LiteralPattern | Uses the supported scalar spellings. Unsupported scalar kinds fail generation. |
| UnitPattern | Backend emits `Nil`; source builtin-unit resolution remains incomplete. |

## Types and versions

All seven type variants have explicit handling. Variables, references, tuples,
functions and unit have mappings. Closed named structural record aliases generate
single-constructor Gleam ADTs. Anonymous nested records and extensible records
fail explicitly. Types, aliases, opaque declarations, generic sum types and
imports have v3/v4 tests in `type_frontend.rs`, `backend_types.rs`,
`ir_versions.rs` and `sum_types.rs`.

The official parser is untyped. This extension does not yet perform Gleam type
inference. Missing function annotations, operator resolution, top-level and
imported value names, and builtin constructors need further work; do not infer
full source-to-IR compatibility from parser acceptance or successful generation.

## Evidence

- `frontend_structural_values.rs` asserts exact IR structure for list patterns,
  tails, sequential bindings, nested blocks, shadowing and terminal assignments.
- `backend_values.rs` compares whole-file goldens, parses output with the official
  parser, covers v3 migration, and verifies explicit failures without artifacts.
- `backend_literals.rs` reparses strings and floats and checks exact contents and
  IEEE bits, including signed zero and large and small values.
- Optional real-Gleam tests in the backend suites type-check representative
  generated modules. The CLI examples compare complete generated source against
  fixed goldens; they do not execute generated functions.

Run `cargo test -p morphir-gleam-binding` for the native suite. With Gleam
installed, run `cargo test -p morphir-gleam-binding --test backend_values --test
backend_literals -- --include-ignored`. WASM parity is checked by
`mise run extension:artifact:gleam`.
