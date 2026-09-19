Feature: Elm in, Elm out
  What this extension compiles it generates again: reading the generated Elm
  yields the very same distribution, in both IR versions.

  The round trip runs with trimmed doc comments, because the Elm printer lays a
  doc comment out itself and so is an inverse of the frontend only for doc text
  that carries no whitespace of its own. Under the default `morphir-elm` mode
  the frontend keeps the raw text, which the printer would then lay out a second
  time — a property of the backend, not of this round trip.

  Scenario Outline: Source, IR, source
    Given the Elm module fixture Types.elm
    And the Elm doc comment option "trimmed"
    When I compile it to Morphir IR version "<version>"
    And I generate Elm from the compiled model
    Then the generated Elm is one artifact per module
    And reading the generated Elm yields the same distribution

    Examples:
      | version |
      | 3       |
      | 4       |
