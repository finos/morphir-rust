Feature: Explicit signed test fixture trust provisioning
  Test artifacts never initialize destination trust without an explicit request.

  Scenario: Export only the signed artifacts by default
    Given a new signed provider fixture destination
    When I export test artifacts without a trust flag
    Then signed interoperability artifacts exist without a trust database

  Scenario: Opt in to fresh test trust for real service acceptance
    Given a new signed provider fixture destination
    When I export with explicit test trust provisioning
    Then the shared service freshly authenticates the exported naming release

  Scenario: Preserve existing test state on repeated export
    Given an explicitly provisioned signed provider fixture
    When I export again to the same destination
    Then the export refuses without changing any existing artifact or trust state

  Scenario: Reject an unknown trust option before creating output
    Given a new signed provider fixture destination
    When I request an unknown trust option
    Then the export refuses without creating the destination
