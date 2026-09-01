Feature: Coordinated automatic cache maintenance
  As a Morphir user
  I want automatic cleanup to load, execute, and save as one transaction
  So that CLI and Desktop processes cannot act on stale maintenance state

  Scenario: Clean registered content and durably record completion
    Given a registered stale cache entry awaiting automatic maintenance
    When I run one automatic maintenance transaction
    Then the registered cache entry is removed
    And the successful automatic run timestamp is durable
