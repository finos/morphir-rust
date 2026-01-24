Feature: Generate Gleam Code from Morphir IR
  As a developer
  I want to generate Gleam source code from Morphir IR
  So that I can target Gleam as a backend

  @codegen-function
  Scenario Outline: Generate function definitions
    Given I have Morphir IR with function definition:
      """
      <ir_function>
      """
    When I generate Gleam code
    Then the generated code should contain:
      """
      <expected_gleam>
      """

    Examples:
      | ir_function                                    | expected_gleam              |
      | {"name": "hello", "body": {...}} | pub fn hello() { ... } |

  @codegen-type
  Scenario Outline: Generate type definitions
    Given I have Morphir IR with type definition:
      """
      <ir_type>
      """
    When I generate Gleam code
    Then the generated code should contain:
      """
      <expected_gleam>
      """

    Examples:
      | ir_type                                    | expected_gleam              |
      | {"name": "Person", "body": {...}} | pub type Person { ... } |
