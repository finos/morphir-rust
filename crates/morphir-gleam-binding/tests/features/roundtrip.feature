Feature: Roundtrip Testing (Gleam → IR V4 → Gleam)
  As a developer
  I want to verify that Gleam code can be roundtripped through IR V4
  So that I can ensure parser and visitor correctness

  @roundtrip @parse
  Scenario: Simple function roundtrip
    Given I have a Gleam source file "input.gleam" with:
      """
      pub fn answer() { 42 }
      """
    When I parse "input.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @parse
  Scenario: String literal roundtrip
    Given I have a Gleam source file "input.gleam" with:
      """
      pub fn hello() { "world" }
      """
    When I parse "input.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @parse
  Scenario: Boolean literal roundtrip
    Given I have a Gleam source file "input.gleam" with:
      """
      pub fn flag() { True }
      """
    When I parse "input.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @parse
  Scenario: Variable reference roundtrip
    Given I have a Gleam source file "input.gleam" with:
      """
      pub fn identity(x) { x }
      """
    When I parse "input.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @type
  Scenario: Simple custom type roundtrip
    Given I have a Gleam source file "input.gleam" with:
      """
      pub type Unit { Unit }
      """
    When I parse "input.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @type
  Scenario: Custom type with variants roundtrip
    Given I have a Gleam source file "input.gleam" with:
      """
      pub type Maybe { Just Nothing }
      """
    When I parse "input.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @real-world @parse-function
  Scenario: Roundtrip rentals fixture with Result type
    Given I load the real-world fixture "rentals.gleam"
    When I parse "rentals.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @real-world @type
  Scenario Outline: Roundtrip real-world union types
    Given I load the real-world fixture "<fixture>"
    When I parse "<module>.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

    Examples:
      | fixture            | module            |
      | order_price.gleam | order_price       |
      | violations.gleam  | violations        |
      | reject_reason.gleam| reject_reason    |
      | rate.gleam        | rate              |

  @roundtrip @real-world @type
  Scenario: Roundtrip complex buy_response fixture
    Given I load the real-world fixture "buy_response.gleam"
    When I parse "buy_response.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @real-world @parse-function @type
  Scenario: Roundtrip order_validation fixture
    Given I load the real-world fixture "order_validation.gleam"
    When I parse "order_validation.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

  @roundtrip @real-world @parse-function @type
  Scenario: Roundtrip order_processing fixture with complete business logic
    Given I load the real-world fixture "order_processing.gleam"
    When I parse "order_processing.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete
