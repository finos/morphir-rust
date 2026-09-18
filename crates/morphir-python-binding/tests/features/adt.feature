Feature: Python algebraic data types
  Scenario Outline: Higher-order functions and closures roundtrip in both IR versions
    Given IR version <version>
    And Python higher-order functions and captured lambdas
    When I compile the model and generate Python
    Then compiling the generated Python preserves the model

    Examples:
      | version |
      | 3       |
      | 4       |

  Scenario: Model declarations survive a Python and Morphir roundtrip
    Given a Python model with a product and a sum with payloads
    When I compile the model and generate Python
    Then compiling the generated Python preserves the model

  Scenario: Unsupported behavior is diagnosed
    Given a Python model containing an unannotated function
    When I compile the model
    Then compilation fails without partial IR

  Scenario: Conditional function bodies survive a Python and Morphir roundtrip
    Given annotated Python functions with conditional bodies
    When I compile the model and generate Python
    Then compiling the generated Python preserves the model

  Scenario: Tuple types and values survive a Python and Morphir roundtrip
    Given Python tuple aliases and tuple-valued functions
    When I compile the model and generate Python
    Then compiling the generated Python preserves the model

  Scenario: Imports preserve types and conditional bodies across modules
    Given Python modules with imported ADTs and tuple aliases
    When I compile the model and generate Python
    Then compiling the generated Python preserves the model

  Scenario Outline: Versioned multi-module models roundtrip
    Given IR version <version>
    And Python modules with imported ADTs and tuple aliases
    When I compile the model and generate Python
    Then compiling the generated Python preserves the model

    Examples:
      | version |
      | 3       |
      | 4       |
