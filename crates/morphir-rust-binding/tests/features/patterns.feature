Feature: Executable Rust pattern matching
  Scenario Outline: Match patterns through either supported IR version
    Given Rust functions matching enums, options, results, tuples and literals
    When I compile the functions to Morphir IR version "<version>"
    And I generate Rust from the compiled model
    Then the generated functions return the expected pattern results

    Examples:
      | version |
      | 3       |
      | 4       |
