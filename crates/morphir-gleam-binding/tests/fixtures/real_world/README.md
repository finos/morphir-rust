# Real-World Test Fixtures

This directory contains Gleam source files adapted from real-world examples in the Morphir ecosystem, specifically from:
- `morphir-examples` repository
- `morphir-elm` repository

## Purpose

These fixtures provide more comprehensive test coverage for the Gleam binding by testing:
- Complex union types with multiple variants
- Types with associated data
- Business logic functions
- Pattern matching
- Record types
- Type aliases (wrapped as custom types where needed)

## Fixture Descriptions

### Simple Types
- **client.gleam**: Simple ID type (adapted from Client.elm)
- **product.gleam**: Product ID type (adapted from Product.elm)
- **price.gleam**: Price type alias (adapted from Price.elm)
- **quantity.gleam**: Quantity type alias (adapted from Quantity.elm)

### Union Types
- **order_price.gleam**: OrderPrice union type with Market and Limit variants
- **violations.gleam**: Violations union type for validation errors
- **reject_reason.gleam**: RejectReason union type for business rejections
- **rate.gleam**: Rate union type with Fee, Rebate, and GC variants

### Records and Complex Types
- **deal.gleam**: Deal record type with id, product, price, quantity
- **buy_response.gleam**: Complex BuyResponse union type with multiple variants
- **order_validation.gleam**: Order validation functions with pattern matching
- **order_processing.gleam**: Complete order processing example with business logic

### Functions
- **rentals.gleam**: Simple rental request function with Result type

## Notes

### Type Aliases
Some fixtures use custom type wrappers instead of true type aliases (e.g., `pub type ID { ID(String) }` instead of `pub type ID = String`) because:
1. The parser may not fully support type alias syntax yet
2. This ensures the fixtures work with the current parser implementation

### Syntax Adaptations
When converting from Elm to Gleam:
- Elm `type alias` → Gleam custom type wrapper (where needed)
- Elm `type` → Gleam `pub type` with variants
- Elm record types → Gleam custom types with constructor
- Elm pattern matching → Gleam `case` expressions
- Elm `Result` → Gleam `Result` (same syntax)

## Source Attribution

All fixtures are adapted from the Morphir examples repository:
- Copyright 2020 Morgan Stanley
- Licensed under the Apache License, Version 2.0
- Original sources: `morphir-examples/src/Morphir/Sample/Apps/`
