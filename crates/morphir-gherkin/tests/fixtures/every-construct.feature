@feature-tag
Feature: Every construct
  Feature description.

  Background: Shared setup
    Given a background step

  Rule: The first rule
    Rule description.

    Background:
      Given a rule background step

    @scenario-tag
    Scenario: Plain scenario
      Given a step with a table
        | a | b |
        | 1 | 2 |
      When a step
      And another step
      Then a result

    Scenario Outline: An outline
      Then <x> is <y>

      @examples-tag
      Examples: Some rows
        | x | y |
        | 1 | 2 |
