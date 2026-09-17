Feature: Python algebraic data types
  Scenario: Model declarations survive a Python and Morphir roundtrip
    Given a Python model with a product and a sum with payloads
    When I compile the model and generate Python
    Then compiling the generated Python preserves the model

  Scenario: Unsupported behavior is diagnosed
    Given a Python model containing a function
    When I compile the model
    Then compilation fails without partial IR
