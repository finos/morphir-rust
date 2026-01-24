# Documentation Code Review Checklist

Use this checklist when reviewing PRs for documentation completeness.

## Public API Documentation

### For New Public Functions/Methods
- [ ] Function has a doc comment explaining its purpose
- [ ] Parameters are documented with their types and meanings
- [ ] Return values are documented
- [ ] Error conditions are documented
- [ ] Example usage is provided for complex APIs

### For New Types
- [ ] Type has a doc comment explaining what it represents
- [ ] Important fields are documented
- [ ] Related types are cross-referenced

### For Breaking Changes
- [ ] Migration guide is provided or updated
- [ ] Changelog entry is added
- [ ] Deprecated APIs are marked and alternatives documented

## User-Facing Features

### For New Features
- [ ] Feature is documented in appropriate user guide
- [ ] Getting started content is updated if applicable
- [ ] Examples demonstrate the feature
- [ ] CLI commands are documented (if applicable)

### For Configuration Options
- [ ] Option is documented with default value
- [ ] Valid values/ranges are specified
- [ ] Example configuration is provided

### For CLI Changes
- [ ] Command help text is clear and accurate
- [ ] CLI docs are updated
- [ ] Examples show common use cases

## Tutorial Review

### Structure
- [ ] Clear learning objectives stated
- [ ] Prerequisites are listed
- [ ] Logical progression of concepts
- [ ] Summary/next steps at the end

### Content Quality
- [ ] Code examples are complete and runnable
- [ ] Steps are numbered and clear
- [ ] Screenshots are current (if included)
- [ ] Links work and point to correct resources

### Accessibility
- [ ] Images have alt text
- [ ] Code blocks specify language
- [ ] Headings follow proper hierarchy

## General Documentation Quality

### Consistency
- [ ] Follows project style guide
- [ ] Uses consistent terminology
- [ ] Matches tone of existing docs
- [ ] Proper Jekyll frontmatter (layout, title, nav_order)

### Links and References
- [ ] Internal links use relative paths
- [ ] External links are valid
- [ ] Links to main Morphir documentation included
- [ ] No orphaned pages (unreachable content)
- [ ] Navigation structure is logical

### Technical Accuracy
- [ ] Code examples have been tested
- [ ] Commands produce expected output
- [ ] Version numbers are correct
- [ ] No outdated information

## Jekyll/GitHub Pages Specific

### Frontmatter
- [ ] `layout: default` is present
- [ ] `title:` is present and descriptive
- [ ] `nav_order:` is set appropriately
- [ ] `parent:` is set if page has a parent
- [ ] `has_children: true` if page has children

### Navigation
- [ ] Navigation order makes sense
- [ ] Parent-child relationships are correct
- [ ] No orphaned pages (pages without navigation)

### Build
- [ ] Documentation builds without errors
- [ ] No broken links
- [ ] All images load correctly
- [ ] Code blocks render properly

## Review Process

### Before Approving
1. **Read the documentation** as if you were a new user
2. **Try the examples** to verify they work
3. **Check cross-references** to ensure they link correctly
4. **Verify Jekyll frontmatter** is correct
5. **Test the build** locally if possible
6. **Consider the audience** - is it appropriate for the target section?

### Common Issues to Watch For
- Broken internal links
- Missing or incorrect Jekyll frontmatter
- Code blocks without language specification
- Outdated screenshots
- Incomplete instructions
- Assumed knowledge not covered by prerequisites
- Missing links to main Morphir documentation

## Documentation Debt Indicators

Flag these for future improvement:
- [ ] TODO comments in documentation
- [ ] Placeholder content
- [ ] "Coming soon" sections
- [ ] Links to non-existent pages
- [ ] Stale API examples
- [ ] Missing links to main Morphir site

## PR Description Requirements

Documentation PRs should include:
- Summary of what was added/changed
- Related issue numbers
- Preview link (if available)
- List of pages affected
- Verification that Jekyll build succeeds
