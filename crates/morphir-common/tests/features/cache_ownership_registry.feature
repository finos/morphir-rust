Feature: Trusted cache ownership
  As a Morphir component
  I want to declare disposable content through a shared durable registry
  So that maintenance removes only content an owner explicitly classified

  Scenario: Clean registered content while preserving unknown siblings
    Given registered and unknown files in a Morphir cache namespace
    When I run cleanup through a guarded ownership session
    Then the registered cache file is removed
    And the unknown cache file remains

  Scenario: Preserve content after its owner releases registration
    Given a cache file whose owner released its registration
    When I run cleanup through a guarded ownership session
    Then the released cache file remains unclassified
