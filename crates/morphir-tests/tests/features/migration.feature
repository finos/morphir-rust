Feature: IR Migration
  As a developer
  I want to migrate IR between format versions
  So that I can upgrade my projects to the new Morphir version

  # Basic Migration Tests

  Scenario: Migrate from Classic to V4
    Given I have a "classic" IR file named "simple_classic.json"
    When I run "morphir ir migrate" to version "v4"
    Then the output file should be a valid "v4" IR distribution
    And the package name should be "test-package"

  Scenario: Migrate from V4 to Classic
    Given I have a "v4" IR file named "simple_v4.json"
    When I run "morphir ir migrate" to version "classic"
    Then the output file should be a valid "classic" IR distribution
    And the package name should be "test-package"

  # V3 to V4 Canonical Name Transformations

  Scenario: Module names should use canonical format in V4
    Given I have a "classic" IR file from fixtures "classic/business-terms.json"
    When I run "morphir ir migrate" to version "v4"
    Then the output file should be a valid "v4" IR distribution
    And all module names should use kebab-case format

  Scenario: Type names should use canonical format in V4
    Given I have a "classic" IR file from fixtures "classic/business-terms.json"
    When I run "morphir ir migrate" to version "v4"
    Then the output file should be a valid "v4" IR distribution
    And all type names should use kebab-case format

  Scenario: Value names should use canonical format in V4
    Given I have a "classic" IR file from fixtures "classic/evaluator-tests.json"
    When I run "morphir ir migrate" to version "v4"
    Then the output file should be a valid "v4" IR distribution
    And all value names should use kebab-case format

  Scenario: Constructor names should use canonical format in V4
    Given I have a "classic" IR file from fixtures "classic/business-terms.json"
    When I run "morphir ir migrate" to version "v4"
    Then the output file should be a valid "v4" IR distribution
    And all constructor names should use kebab-case format

  # V3 to V4 Type Expression Transformations

  Scenario: Reference types should use object wrapper format in V4
    Given I have a "classic" IR file from fixtures "classic/business-terms.json"
    When I run "morphir ir migrate" to version "v4"
    Then the output file should be a valid "v4" IR distribution
    And type references should use the V4 object wrapper format
    And FQNames should use canonical format

  Scenario: Record types should preserve fields in V4 format
    Given I have a "classic" IR file from fixtures "classic/evaluator-tests.json"
    When I run "morphir ir migrate" to version "v4"
    Then the output file should be a valid "v4" IR distribution
    And record type fields should use kebab-case names

  Scenario: Value definitions should have properly converted content in V4
    Given I have a "classic" IR file from fixtures "classic/evaluator-tests.json"
    When I run "morphir ir migrate" to version "v4"
    Then the output file should be a valid "v4" IR distribution
    And value definitions should have non-null body content
    And value definitions should have properly converted inputTypes
    And value definitions should have properly converted outputType

  # Like-for-like roundtrip tests

  Scenario: Classic to V4 to Classic roundtrip preserves structure
    Given I have a "classic" IR file from fixtures "classic/business-terms.json"
    When I run "morphir ir migrate" to version "v4"
    And I save the result as intermediate
    And I run "morphir ir migrate" on intermediate to version "classic"
    Then the output file should be a valid "classic" IR distribution

  Scenario: V4 to Classic to V4 roundtrip preserves structure
    Given I have a "v4" IR file named "simple_v4.json"
    When I run "morphir ir migrate" to version "classic"
    And I save the result as intermediate
    And I run "morphir ir migrate" on intermediate to version "v4"
    Then the output file should be a valid "v4" IR distribution
