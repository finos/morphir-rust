# Agent Guidance

> [!NOTE]
> This document serves as the primary guidance for AI Agents working on the Morphir Rust ecosystem.

## Morphir Ecosystem

AI agents working on this project are expected to be well-versed in Morphir concepts, the Morphir IR, and related tools, designs, specifications, and schemas.

### Essential Resources
- **Official Documentation**: [https://morphir.finos.org](https://morphir.finos.org) - Comprehensive Morphir documentation, concepts, and guides
- **LLM Context File**: [https://morphir.finos.org/llms.txt](https://morphir.finos.org/llms.txt) - AI agent-optimized context about Morphir
- **Reference Implementation**: [finos/morphir-elm](https://github.com/finos/morphir-elm) - The canonical Elm implementation of Morphir
- **GitHub Organization**: [github.com/finos](https://github.com/finos) - Browse other Morphir ecosystem projects:
  - `morphir-elm` - Reference implementation and IR specification
  - `morphir-jvm` - JVM/Scala implementation
  - `morphir-dotnet` - .NET implementation
  - `morphir-examples` - Example Morphir models and use cases
  - Additional language bindings and tooling

### Expected Knowledge
Agents should understand:
- **Morphir IR**: The intermediate representation structure, versioning, and evolution
- **Morphir Concepts**: Functional domain modeling, type systems, and distribution patterns
- **IR Schemas**: JSON schemas, serialization formats, and compatibility requirements
- **Tooling Ecosystem**: How different Morphir implementations and tools interact

When uncertain about Morphir-specific design decisions, consult the reference implementation and official documentation.

## Technology Stack

### Core Technologies
- **Language**: Rust (Edition 2021)
- **Build System**: Cargo
- **Async Runtime**: `tokio`

### Key Libraries & Frameworks
- **CLI Framework**: `starbase` (application structure), `clap` (command parsing)
- **TUI**: `ratatui` (terminal UI widgets), `crossterm` (cross-platform terminal manipulation), `tuirealm` (component framework)
- **Serialization**: `serde` & `serde_json` (JSON handling), `schemars` (JSON Schema generation)
- **Logging**: `tracing` (structured logging), `tracing-subscriber` (log subscribers), `tracing-appender` (file logging)
- **Error Handling**: `thiserror` (library errors), `anyhow` (application/CLI errors)

## Coding Standards & Best Practices

### Documentation & Examples
- **Public APIs**: All public types, functions, and modules must have documentation comments (`//!` for modules, `///` for items).
- **Examples Required**: Components providing significant user behavior (such as loaders, visitors, and complex converters/transformers) **must** include doc tests/examples demonstrating usage.

### Testing Strategy
- **TDD & BDD**: We prioritize robust testing and reliability.
    - **Test-Driven Development (TDD)**: Write tests before implementation to ensure correctness and design quality.
    - **Behavior-Driven Development (BDD)**: For complex workflows and behaviors (like IR migration or CLI commands), use `cucumber` with Gherkin feature files (`.feature`) to describe compliance and behavior in plain English.
- **Coverage**: Ensure high test coverage for core logic.
- **Object Mother Pattern**: Use the Object Mother pattern to create well-named factory functions that produce test data for specific scenarios:
    - Create functions like `a_simple_package()`, `a_package_with_dependencies()`, `an_invalid_ir_definition()` that return pre-configured test data
    - Place these in a dedicated `test_data` or `mothers` module within your tests
    - Makes tests more readable and reduces duplication of complex setup code
    - Facilitates testing edge cases and specific scenarios consistently
- **BDD Test Drivers**: All BDD/Cucumber tests **must** have a corresponding TestDriver:
    - Create a `TestDriver` struct that encapsulates the system under test and its dependencies
    - The driver provides domain-specific methods that map to Gherkin steps (e.g., `when_migrating_ir()`, `then_output_should_contain()`)
    - Keeps step definitions clean and delegates complex setup/assertions to the driver
    - Example: `tests/drivers/migration_driver.rs` for IR migration BDD tests
- **Whitebox & Blackbox Testing**: Use both testing approaches to validate proper encapsulation:
    - **Blackbox Tests**: Test public APIs without knowledge of internal implementation. Validates that the public interface works correctly and is stable. Place these in integration tests or BDD scenarios.
    - **Whitebox Tests**: Test internal implementation details, edge cases, and private functions. Use `#[cfg(test)]` modules within source files for unit tests that need access to private items.
    - **Balance**: Prefer blackbox tests for API stability; use whitebox tests to ensure correctness of internal logic and edge cases that are hard to trigger through the public API.
    - **Encapsulation Validation**: If internal implementation details leak into blackbox tests, this indicates poor encapsulation that should be refactored.

### Functional Design Principles
Reflecting Morphir's functional domain-driven design nature, strictly adhere to:
- **Algebraic Data Types (ADTs)**: Design domain models using ADTs to precisely capture the problem domain:
    - **Product Types**: Use structs to combine multiple values (e.g., `struct Package { name: PackageName, modules: Vec<Module> }`)
    - **Sum Types**: Use enums to represent alternatives and variants (e.g., `enum Value { Literal(Literal), Constructor(FQName), Apply { function: Box<Value>, argument: Box<Value> } }`)
    - **Pattern Matching**: Leverage exhaustive pattern matching to ensure all cases are handled
    - **Type-Driven Design**: Let the type system guide implementation and catch errors at compile time
- **Immutability**: Prefer immutable data structures.
- **Illegal States Unrepresentable**: Design types such that invalid states cannot be constructed (e.g., use Enums/Sum Types over boolean flags + optional fields).
- **No Primitive Obsession**: Wrap primitives in domain types (e.g., `PackageName` struct instead of `String`).
- **Composition**: Build complex logic by composing small, pure functions.
- **Strong Cohesion & Loose Coupling**: Keep related logic together; minimize dependencies between distinct domains.

### Code Organization & Module Structure
- **Avoid Mega Files**: Do not create large, monolithic source files. Split code into focused, cohesive modules.
- **Module-per-Directory Pattern**: When a module grows beyond ~300-500 lines or contains multiple related types:
  - Create a directory with the module name
  - Use `mod.rs` to expose the public API
  - Split implementation into separate files by logical grouping (e.g., `types.rs`, `parser.rs`, `visitor.rs`)
- **File Size Guidelines**:
  - Single-purpose files: ~200-400 lines is ideal
  - Complex implementations: Consider splitting at 500+ lines
  - If a file has multiple `impl` blocks for different concerns, split by concern
- **Logical Grouping**: Organize files by:
  - **Domain concepts**: Group related types and their implementations together
  - **Layers**: Separate concerns like parsing, transformation, validation, serialization
  - **Visibility**: Private helpers can live in submodules; public API should be clear in `mod.rs`
- **Example Structure**:
  ```
  src/
    ir/
      mod.rs          // Public API, re-exports
      types.rs        // Core type definitions
      parser.rs       // Parsing logic
      visitor.rs      // Visitor implementations
      transform/      // Nested module for transformations
        mod.rs
        migration.rs
        optimization.rs
  ```

### Git Workflow & Version Control
- **Commit Upon Task Completion**: Create a commit as soon as a logical unit of work is complete (e.g., after implementing a feature, fixing a bug, or completing a refactoring).
- **Avoid Large Worktrees**: Do not accumulate large amounts of uncommitted changes. Break work into smaller, committable units.
- **Atomic Commits**: Each commit should represent a single, coherent change that:
  - Compiles successfully (if applicable to the change)
  - Passes existing tests (or updates tests appropriately)
  - Can be understood independently
- **Commit Message Quality**: Write clear, descriptive commit messages:
  - First line: Brief summary (50-72 characters) in imperative mood
  - Body: Explain *why* the change was made, not just *what* changed
  - Reference related issues/beads when applicable
- **AI Agent Authorship - CLA Restriction**: **NEVER** add AI agents (e.g., Claude, ChatGPT, etc.) as the author or co-author in git commits. Due to FINOS Contributor License Agreement (CLA) requirements, only human contributors with signed CLAs may be listed as authors or co-authors. Commits must be attributed solely to the human developer directing the work.
- **Use Git for History**: Rely on git history to track changes and evolution—don't keep commented-out code or version suffixes in filenames.

### Temporary Output Management
- **No Repository Littering**: Do not create temporary output files in the repository root or source directories.
- **Gitignored Locations**: Direct all temporary outputs (including redirected command output, test artifacts, and working files) to gitignored directories:
  - **Preferred**: `.agents/out/` (for agent-generated temporary files)
  - **Alternative**: `.morphir/out/` (for Morphir-specific outputs)
- **Redirect Output**: When running commands that generate output files, explicitly redirect them to the gitignored locations above rather than letting them write to the working directory.
- **Clean Working Directory**: Keep the git working directory clean to avoid accidental commits of temporary files.

### Logging Standards
- **stdout is Reserved for Output**: Never write logs to stdout. stdout is exclusively for actual program output (command results, generated content, data). This ensures:
  - CLI output can be piped to other commands
  - JSON/structured output remains parseable
  - Scripts can capture actual results without log noise
- **stderr for Console Logging**: All console-level logging (warnings, errors, progress messages) must go to stderr.
- **File-Based Logging**: Log files should be written to:
  - **Workspace-local**: `.morphir/logs/` when inside a morphir workspace (a directory containing `morphir.toml` or `.morphir/`)
  - **Global fallback**: `~/.morphir/logs/` when not in a workspace
- **Log Levels**: Use appropriate log levels:
  - `error`: Unrecoverable failures
  - `warn`: Recoverable issues or deprecation notices
  - `info`: High-level progress indicators (default for CLI)
  - `debug`: Detailed operational information
  - `trace`: Highly verbose debugging output
- **Structured Logging**: Prefer structured log formats (JSON lines) for file-based logs to enable analysis and aggregation.
- **Rotation**: File logs should support rotation to prevent unbounded growth.
- **Configuration**: Log destinations and levels should be configurable via:
  - Environment variables: `MORPHIR_LOG_LEVEL`, `MORPHIR_LOG_DIR`
  - Config file: `[logging]` section in `morphir.toml`

## CLI Documentation Generation

The CLI reference documentation is auto-generated from the Rust CLI source code via clap's usage output.

### Key Files
- **Source of Truth**: `crates/morphir/src/main.rs` - Rust source with clap derive macros
- **Intermediate**: `docs/morphir.usage.kdl` - Generated from `morphir usage` command
- **Generated Output**: `docs/cli/` directory - Markdown files generated from the KDL spec
- **Task**: `.mise/tasks/docs/markdown` - Orchestrates generation and post-processing

### Regenerating CLI Docs

```bash
mise run docs:markdown    # Regenerate CLI docs
mise run docs:generate    # Regenerate all docs (CLI + release notes)
```

The task automatically:
1. Builds the morphir binary
2. Generates `docs/morphir.usage.kdl` from `morphir usage`
3. Generates markdown with `usage generate markdown`
4. Adds Jekyll front matter for just-the-docs theme

### Adding Examples to CLI Docs

**DO NOT** add examples directly to generated markdown files or the KDL file - they will be lost on regeneration.

Instead, add a `long_about` attribute in the Rust source code (`crates/morphir/src/main.rs`):

```rust
#[derive(Clone, Subcommand)]
enum IrAction {
    /// Migrate IR between versions
    #[command(long_about = "Migrate IR between versions

Converts Morphir IR between Classic (V1-V3) and V4 formats.

**Examples:**

```bash
morphir ir migrate ./morphir-ir.json -o ./morphir-ir-v4.json
```

See the [IR Migration Guide](/ir-migrate/) for details.")]
    Migrate {
        // fields...
    }
}
```

For detailed real-world examples (like step-by-step walkthroughs), add them to separate guide pages in `docs/` that are NOT auto-generated (e.g., `docs/ir-migrate.md`).

### Clap Attributes for Documentation
- Doc comment (`///`) - Short description shown in command listing
- `#[command(long_about = "...")]` - Extended help with examples (supports markdown)
- `#[command(hide = true)]` - Hide command from default help (experimental commands)
- `#[arg(short, long)]` - Short and long flag names
- `#[arg(required = true)]` - Mark argument as required

### Markdown Console Rendering
The CLI includes `termimad` for rendering markdown to the terminal with proper syntax highlighting. Use `help::print_markdown()` in `crates/morphir/src/help.rs` to render markdown text.

## Release Management

### Overview
morphir-rust uses a comprehensive release management system:
- **CHANGELOG.md**: Follows [keepachangelog](https://keepachangelog.com) format
- **Mise tasks**: `mise run release:*` for release workflow automation
- **Claude skill**: `/release-manager` for AI-assisted release operations

### Changelog Guidelines
- All notable changes must be documented in `CHANGELOG.md`
- Add entries to `[Unreleased]` as changes are made
- Use categories: Added, Changed, Deprecated, Removed, Fixed, Security
- Reference PRs/issues where applicable

### Release Tasks
| Task | Description |
|------|-------------|
| `mise run release:check` | Run all pre-release quality checks |
| `mise run release:version-bump <v>` | Bump workspace version |
| `mise run release:changelog-validate` | Validate CHANGELOG.md format |
| `mise run release:changelog-entry <cat> <msg>` | Add changelog entry |
| `mise run release:pre-release <v>` | Full pre-release workflow |
| `mise run release:tag-create <v> [--push]` | Create release tag |
| `mise run release:post-release <v>` | Post-release validation |

### Release Process Quick Reference
```bash
# 1. Run pre-release checks
mise run release:pre-release 0.2.0

# 2. Bump version and update changelog
mise run release:version-bump 0.2.0
# Edit CHANGELOG.md manually

# 3. Commit and tag
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore: prepare release v0.2.0"
mise run release:tag-create 0.2.0 --push

# 4. Monitor CI and publish release on GitHub
```

### AI-Assisted Releases
Use the `/release-manager` skill for guided release workflows:
- `/release-manager prepare <version>` - Pre-release preparation
- `/release-manager analyze` - Analyze commits for changelog
- `/release-manager retrospective <version>` - Post-release review
- `/release-manager whats-new <version>` - Generate release summaries

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
