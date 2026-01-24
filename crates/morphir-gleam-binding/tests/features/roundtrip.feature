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
