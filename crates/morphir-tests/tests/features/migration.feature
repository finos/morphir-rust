Feature: IR Migration
  As a developer
  I want to migrate IR between format versions
  So that I can upgrade my projects to the new Morphir version

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
