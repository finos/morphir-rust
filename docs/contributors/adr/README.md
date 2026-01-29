# Architecture Decision Records (ADR)

This directory contains Architecture Decision Records for the Morphir Rust implementation.

## What is an ADR?

An Architecture Decision Record (ADR) captures an important architectural decision made along with its context and
consequences.

## Format

Each ADR follows this structure:

- **Title**: Brief description of the decision
- **Status**: Proposed, Accepted, Deprecated, Superseded
- **Date**: When the decision was made
- **Context**: The problem or situation requiring a decision
- **Decision**: What was decided
- **Rationale**: Why this decision was made
- **Consequences**: Trade-offs, impacts, follow-up work
- **Alternatives Considered**: Other options and why they were rejected

## Index

| ADR                                                    | Title                                          | Status   | Date       |
|--------------------------------------------------------|------------------------------------------------|----------|------------|
| [0001](./0001-envelope-only-execution-for-builtins.md) | Envelope-Only Execution for Builtin Extensions | Accepted | 2026-01-29 |

## Creating a New ADR

1. Copy the template (if one exists) or follow the format of existing ADRs
2. Use the next sequential number: `XXXX-brief-description.md`
3. Fill in all sections with context and reasoning
4. Include diagrams or code examples where helpful
5. Update this README index
6. Commit with message: `docs: Add ADR-XXXX [brief description]`

## ADR Lifecycle

- **Proposed**: Decision is being discussed
- **Accepted**: Decision has been made and is in effect
- **Deprecated**: Decision is no longer recommended but not replaced
- **Superseded**: Decision has been replaced by a newer ADR (reference it)

## When to Create an ADR

Create an ADR when:

- Making a significant architectural choice
- Choosing between multiple viable alternatives
- Setting a pattern or convention for the codebase
- Making a decision that will be hard to reverse
- Answering "why did we do it this way?" questions

## References

- [ADR GitHub Organization](https://adr.github.io/)
- [Documenting Architecture Decisions](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions) by
  Michael Nygard
