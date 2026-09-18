Feature: Elm type declarations through Morphir
  An Elm module of type declarations compiles to every supported Morphir IR
  version. Value declarations are not part of this frontend's subset, so each
  one is reported as skipped rather than silently dropped.

  Scenario Outline: Compile a module of type declarations
    Given the Elm module fixture Types.elm
    When I compile it to Morphir IR version "<version>"
    Then the IR contains type "Account" as an alias with 1 type parameter
    And the IR contains custom type "Status" with constructors "Active, Closed, Pending"
    And value "greet" is reported as skipped

    Examples:
      | version |
      | 3       |
      | 4       |
