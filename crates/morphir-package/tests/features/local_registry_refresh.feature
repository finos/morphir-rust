Feature: Explicit metadata-only local registry refresh
  Scenario: Repeated refresh observes exact metadata without changing locks
    Given a provisioned signed registry
    When I explicitly refresh metadata
    Then only metadata observations are returned and the lock is unchanged
    When I explicitly refresh metadata
    Then the exact metadata digests repeat

  Scenario: Package availability does not control metadata refresh
    Given a provisioned signed registry
    And all package material is missing
    When I explicitly refresh metadata
    Then only metadata observations are returned and the lock is unchanged

  Scenario: Equal timestamps still require the complete current chain
    Given a provisioned signed registry
    When I explicitly refresh metadata
    And the current child metadata is corrupted
    And I explicitly refresh metadata
    Then refresh refuses and retains its operation marker
    And a restarted refresh refuses the unresolved operation

  Scenario: Authenticated revocation cannot be forgotten
    Given a provisioned registry with an authenticated revoked declaration
    And all package material is missing
    When I explicitly refresh metadata
    Then refresh refuses the unsupported revocation transition
    And a restarted refresh refuses the unresolved operation
