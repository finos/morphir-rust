Feature: One-shot authenticated snapshot service
  The host qualifies the executable and grants its roots. Every snapshot request
  authenticates freshly and exports retained original bytes.

  Background:
    Given a signed naming service fixture with provisioned trust

  Scenario: An explicit probe has no restore effects
    When I probe the executable without restore inputs
    Then the executable returns capabilities without filesystem effects

  Scenario: A fresh signed snapshot crosses the binary boundary
    When I request the exact locked naming release
    Then one binary response retains the original naming inventory

  Scenario: A different root cannot borrow the full lock
    When I request a different release
    Then the executable refuses the release without output or publication

  Scenario: A previous response cannot authorize another operation
    When I request the exact locked naming release
    Then one binary response retains the original naming inventory
    When the current timestamp becomes invalid
    And I request the exact locked naming release
    Then the executable refuses fresh authentication without output or publication
