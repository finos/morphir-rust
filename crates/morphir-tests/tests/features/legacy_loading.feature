Feature: Legacy IR Loading
  I want to load older versions of Morphir IR (V1, V2, V3)
  So that I can support existing projects without immediate migration

  Scenario Outline: Load Legacy IR Version
    Given I have a "classic" IR file named "<filename>"
    When I load the distribution from the file
    Then I should get a valid "classic" IR distribution
    And the package name should be "<package_name>"

    Examples:
      | filename            | package_name            |
      | real_v1.json        | morphir-example-app     |
      | real_v2.json        | morphir-example-app     |
      | real_v3.json        | morphir-reference-model |
      | morphir_elm_v3.json | morphir-elm             |
      | lcr_v3.json         | finos-lcr               |
