Feature: Roundtrip Testing (Gleam → IR V4 → Gleam)
  As a developer
  I want to verify that Gleam code can be roundtripped through IR V4
  So that I can ensure parser and visitor correctness

  @roundtrip-function @parse-function @visitor-value @codegen-function
  Scenario Outline: Roundtrip <construct_type>
    Given I have a Gleam source file "input.gleam" with:
      """
      <gleam_source>
      """
    When I parse "input.gleam" to ModuleIR
    And I convert ModuleIR to IR V4 Document Tree
    Then the roundtrip should complete

    Examples: Simple constructs
      | construct_type | gleam_source                    |
      | function       | pub fn add(x, y) { x + y }     |
      | type           | pub type Point { Point(Int, Int) } |
