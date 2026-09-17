Feature: Python algebraic data types
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
