Feature: Parse Gleam Source Files
  As a developer
  I want to parse Gleam source files to ModuleIR
  So that I can convert them to Morphir IR

  @parse-function
  Scenario Outline: Parse basic Gleam constructs
    Given I have a Gleam source file "<file>" with:
      """
      <source>
      """
    When I parse the file
    Then parsing should succeed
    And the parsed module should have name "<module_name>"
    And the parsed module should have <type_count> type definitions
    And the parsed module should have <value_count> value definitions

    Examples:
      | file                    | source                          | module_name | type_count | value_count |
      | simple_function.gleam  | pub fn hello() { "world" }     | simple_function | 0      | 1           |
      | with_types.gleam       | pub type Person { Person }      | with_types  | 1          | 0           |

  @parse-function @parse-type
  Scenario Outline: Parse function with type annotation
    Given I have a Gleam source file with:
      """
      pub fn add(x: Int, y: Int) -> Int {
        x + y
      }
      """
    When I parse the file
    Then parsing should succeed
    And the parsed module should have 1 value definitions
