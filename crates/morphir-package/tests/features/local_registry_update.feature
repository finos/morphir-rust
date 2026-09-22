Feature: Freshly authenticated scoped Library update
  Scenario: Eligible update moves only its old dependency closure
    Given a provisioned current registry and historical four-Library lock
    When I update the eligibility dependency
    Then only the target and necessary child move in the independent full lock
    And all four updated Libraries can be restored

  Scenario: Exact frozen yanked sibling remains usable
    Given a current registry with a yanked frozen sibling
    When I update the eligibility dependency
    Then the yanked sibling remains pinned in the independent full lock
    And all four updated Libraries can be restored

  Scenario: Scope conflict leaves no new lock and blocks restart
    Given a current registry whose exact target requires changing a frozen sibling
    When I update the eligibility dependency
    Then the update refuses the scope conflict without changing the old lock
    And another update refuses the unresolved operation
