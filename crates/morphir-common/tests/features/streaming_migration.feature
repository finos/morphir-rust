Feature: Streaming IR migration
  As a Morphir tool author
  I want migration to compose concrete codecs through semantic events
  So that large models do not require whole-distribution retention

  Scenario: Stream concrete Classic v3 JSON to native v4 YAML
    Given a concrete Classic v3 JSON distribution
    When I stream it through the v3 to v4 pipeline into native YAML
    Then the migration output is concrete v4 YAML
    And the migration pipeline retains at most one module
    And the migration report permits publication
