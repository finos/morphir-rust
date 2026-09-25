Feature: Step libraries link across crates
  Scenario: A step defined in the library runs here
    Given the step library is linked
    Then the linked flag is set

  Scenario: A missing step fails the scenario
    Given a step that no library defines
