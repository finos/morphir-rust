Feature: Fresh authenticated local Library resolution
  A generated lock is usable only after the complete graph has been verified.

  Background:
    Given a provisioned signed two-Library registry

  Scenario: Resolve and consume the complete lock
    When I resolve the published Library root
    Then the complete draft.3 lock is published
    When I restore the generated lock
    Then both Libraries are materialized

  Scenario: Repeated resolution is deterministic
    When I resolve the published Library root
    And I resolve the same root to another lock file
    Then the generated locks are byte-identical

  Scenario: Existing output is never replaced
    Given the lock destination already contains a file
    When I resolve the published Library root
    Then the existing lock bytes are unchanged
    And no trust operation was started

  Scenario: A corrupt provider prevents graph readiness
    Given the provider content is corrupt
    When I resolve the published Library root
    Then no lock is published
    And another resolution attempt refuses the unresolved operation
