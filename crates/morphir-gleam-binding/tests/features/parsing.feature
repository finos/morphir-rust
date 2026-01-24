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

  @parse-real-world @parse-function
  Scenario: Parse rentals fixture with Result type
    Given I load the real-world fixture "rentals.gleam"
    When I parse the file
    Then parsing should succeed
    And the parsed module should have name "rentals"
    And the parsed module should have 0 type definitions
    And the parsed module should have 1 value definitions

  @parse-real-world @parse-type
  Scenario Outline: Parse real-world type aliases
    Given I load the real-world fixture "<fixture>"
    When I parse the file
    Then parsing should succeed
    And the parsed module should have name "<module>"
    And the parsed module should have 1 type definitions
    And the parsed module should have 0 value definitions

    Examples:
      | fixture        | module   |
      | client.gleam  | client   |
      | product.gleam | product  |
      | price.gleam   | price    |
      | quantity.gleam| quantity |

  @parse-real-world @parse-type
  Scenario Outline: Parse real-world union types
    Given I load the real-world fixture "<fixture>"
    When I parse the file
    Then parsing should succeed
    And the parsed module should have name "<module>"
    And the parsed module should have <type_count> type definitions
    And the parsed module should have 0 value definitions

    Examples:
      | fixture            | module            | type_count |
      | order_price.gleam  | order_price       | 1          |
      | violations.gleam   | violations        | 1          |
      | reject_reason.gleam| reject_reason     | 1          |
      | rate.gleam         | rate              | 1          |
      | deal.gleam         | deal              | 1          |

  @parse-real-world @parse-type
  Scenario: Parse complex buy_response fixture
    Given I load the real-world fixture "buy_response.gleam"
    When I parse the file
    Then parsing should succeed
    And the parsed module should have name "buy_response"
    And the parsed module should have 3 type definitions
    And the parsed module should have 0 value definitions

  @parse-real-world @parse-function @parse-type
  Scenario: Parse order_validation fixture with functions and types
    Given I load the real-world fixture "order_validation.gleam"
    When I parse the file
    Then parsing should succeed
    And the parsed module should have name "order_validation"
    And the parsed module should have 3 type definitions
    And the parsed module should have 2 value definitions

  @parse-real-world @parse-function @parse-type
  Scenario: Parse order_processing fixture with complete business logic
    Given I load the real-world fixture "order_processing.gleam"
    When I parse the file
    Then parsing should succeed
    And the parsed module should have name "order_processing"
    And the parsed module should have 5 type definitions
    And the parsed module should have 3 value definitions
