# Spec/Design Consistency Checklist

Use this checklist when reviewing specification documents against design documents in `docs/contributors/design/`.

## Naming Conventions

### Canonical String Format Consistency
- [ ] FQName examples use correct format: `package/path:module/path#local-name`
- [ ] Path examples use correct format: `segment/segment` (not mixed with FQName separators)
- [ ] Name examples use correct format: `kebab-case` with `(abbreviations)` for letter sequences
- [ ] No confusion between paths and FQNames (paths don't have `:` or `#`)

### Common Naming Errors
| Error | Example | Correct |
|-------|---------|---------|
| Path with FQName separators | `morphir/sdk:string` | `morphir/sdk` (path) or `morphir/sdk:string#string` (FQName) |
| Missing local name in FQName | `morphir/sdk:list` | `morphir/sdk:list#map` |
| Abbreviation not marked | `morphir/sdk` | `morphir/(sdk)` only if S-D-K, else `morphir/sdk` for word "sdk" |

## Type/Value Node Consistency

### Type Expressions
- [ ] All type nodes from design are documented in spec
- [ ] Structure notation matches design (e.g., `Type attributes name` vs `Type(attributes, name)`)
- [ ] Components list matches design fields
- [ ] v4-specific nodes are marked with `(v4)`

### Value Expressions
- [ ] All value nodes from design are documented in spec
- [ ] v4 additions present: `Hole`, `Native`, `External`
- [ ] `ValueDefinitionBody` variants documented: `ExpressionBody`, `NativeBody`, `ExternalBody`, `IncompleteBody`

### Specifications vs Definitions
- [ ] Clear distinction explained between specs and definitions
- [ ] All spec types listed: `TypeAliasSpecification`, `OpaqueTypeSpecification`, `CustomTypeSpecification`, `DerivedTypeSpecification`
- [ ] All definition types listed: `TypeAliasDefinition`, `CustomTypeDefinition`, `IncompleteTypeDefinition`
- [ ] Derivation rules documented (definition → specification)

## JSON Serialization Consistency

### Format Examples
- [ ] JSON examples are valid JSON
- [ ] Wrapper object format matches design: `{ "NodeType": { "field": value } }`
- [ ] Shorthand formats documented where applicable
- [ ] Legacy format compatibility documented

### Field Naming
- [ ] JSON field names match design (camelCase vs snake_case)
- [ ] Required vs optional fields match design
- [ ] Default values documented where applicable

### Schema Alignment
- [ ] JSON Schema matches design doc examples
- [ ] Distribution variants match: Library, Specs, Application all present
- [ ] Distribution uses wrapper object format: `{ "Library": { ... } }` not array format
- [ ] Field names match design
- [ ] Dependencies use object/dict format, not array of tuples

### Schema Documentation and Examples
- [ ] All schema definitions have clear, descriptive `description` fields
- [ ] Key definitions include `examples` arrays with realistic, valid JSON examples
- [ ] Examples demonstrate V4 wrapper object format where applicable
- [ ] Examples show both canonical and shorthand formats when supported
- [ ] Complex structures have complete examples
- [ ] Examples are consistent with design document examples

## Directory Structure Consistency

### File Naming Patterns
- [ ] Document Tree file suffixes are consistent: `.type.json`, `.value.json`, `module.json`
- [ ] Directory examples use canonical name format (kebab-case)
- [ ] Path separators are forward slashes `/`

### Expected Directory Structure

Package and module paths expand to fully split directories. Definition files (`.type.json`, `.value.json`) reside directly in the module directory—the suffixes distinguish types from values.

```
pkg/my-org/my-project/
└── orders/
    ├── module.json
    ├── order.type.json
    ├── line-item.type.json
    ├── create-order.value.json
    ├── calculate-total.value.json
    └── shipping/
        ├── module.json
        ├── address.type.json
        └── calculate-cost.value.json
```

## Cross-Reference Validation

### Internal Links
- [ ] References to other spec documents use correct paths
- [ ] References to design documents are accurate
- [ ] No dangling references to removed/renamed concepts
- [ ] Links to main Morphir documentation are included

### Terminology
- [ ] Terms used consistently across spec and design
- [ ] No conflicting definitions of the same concept
- [ ] Abbreviations/acronyms explained on first use

## Review Process

1. **Open both documents side-by-side** - spec and corresponding design
2. **Walk through design section by section** - verify each concept in spec
3. **Validate all examples** - copy-paste and check they parse
4. **Check schema documentation** - ensure schemas have descriptions and examples
5. **Check cross-references** - ensure links resolve correctly
6. **Note discrepancies** - create issues or fix directly
7. **Generate review document (optional)** - save to `.morphir/out/` for local reference
   - Review documents should NOT be committed to git
   - Use `.morphir/out/` directory (gitignored) for review outputs
   - Review documents are for tracking progress and findings locally
