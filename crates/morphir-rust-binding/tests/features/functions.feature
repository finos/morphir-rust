Feature: Executable Rust functions and lambdas
  Scenario Outline: Calls and Copy captures through either supported IR version
    Given Rust functions using calls and typed lambdas with Copy captures
    When I compile the functions to Morphir IR version "<version>"
    And I generate Rust from the compiled model
    Then the generated functions return the expected callable results

    Examples:
      | version |
      | 3       |
      | 4       |
