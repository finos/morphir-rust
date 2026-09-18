Feature: Elm in, Elm out
  What this extension compiles it generates again: reading the generated Elm
  yields the very same distribution, in both IR versions.

  Scenario Outline: Source, IR, source
    Given the Elm module fixture Types.elm
    When I compile it to Morphir IR version "<version>"
    And I generate Elm from the compiled model
    Then the generated Elm is one artifact per module
    And reading the generated Elm yields the same distribution

    Examples:
      | version |
      | 3       |
      | 4       |
