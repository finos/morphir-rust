Feature: Elm compilation diagnostics
  What the frontend refuses, it explains: every diagnostic that concerns source
  carries the code and the place a reader has to look.

  Scenario: A syntax error is reported where it is
    Given an Elm module that does not parse
    When I compile it to Morphir IR version "3"
    Then compilation fails
    And diagnostic "ELM_SYNTAX" is reported at line 2 of "file:///work/Invalid.elm"

  Scenario: An unresolved name names the prelude it was looked up in
    Given an Elm module aliasing Int
    And the Elm prelude option "none"
    When I compile it to Morphir IR version "3"
    Then compilation fails
    And diagnostic "ELM_RESOLVE_NOT_FOUND" mentions "prelude: none"

  Scenario: A name two imports both expose is ambiguous
    Given two imported modules that both expose the type T
    When I compile it to Morphir IR version "3"
    Then compilation fails
    And diagnostic "ELM_RESOLVE_AMBIGUOUS" mentions "T"
