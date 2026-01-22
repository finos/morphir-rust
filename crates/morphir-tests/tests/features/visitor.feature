Feature: Visitor Traversal
  I want to use the Visitor pattern to traverse the Morphir IR
  So that I can analyze or transform the IR structure

  Scenario: Count Modules in a Distribution
    Given I have a "classic" IR file named "simple_classic.json"
    When I visit the distribution using a Module Counting Visitor
    Then the module count should be 1

  Scenario: Count Variables in an Expression
    Given I have a simple expression with 3 variables
    When I visit the expression using a Variable Counting Visitor
    Then the variable count should be 0
