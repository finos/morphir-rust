Feature: Packaged provider snapshot release qualification
  A release ships the production snapshot service and the test-only fixture
  exporter as separate executables. Before and after publication, the exact
  packaged executables run as child processes. Every scenario exports new signed
  test artifacts and fresh test trust into its own new directory.

  Background:
    Given the packaged service and test fixture executables

  Scenario: The packaged service reports its protocol without restore inputs
    When I probe the packaged service
    Then the packaged service reports the snapshot profile

  Scenario: The packaged exporter creates no trust database by default
    When I export packaged test artifacts without a trust flag
    Then the export contains signed artifacts without a trust database

  Scenario: The packaged exporter refuses an unknown option before any output
    When I export packaged test artifacts with an unknown option
    Then the exporter fails without creating the destination

  Scenario Outline: The packaged service freshly authenticates each signed variant
    When I export the packaged "<variant>" variant with fresh test trust
    And I request the naming release from the packaged service
    Then the response retains the exported "<variant>" bytes and checked core

    Examples:
      | variant            |
      | compatible         |
      | wrong-type         |
      | wrong-subject-role |
      | wrong-object-form  |
      | wrong-interpreter  |
      | ambiguous-contexts |

  Scenario: The packaged service refuses invalid current metadata
    When I export the packaged "compatible" variant with fresh test trust
    And the exported timestamp becomes invalid
    And I request the naming release from the packaged service
    Then the packaged service refuses authentication without output
