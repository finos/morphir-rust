Feature: Rust type models through Morphir
  Rust domain models can be compiled to supported Morphir IR versions and
  generated as Rust that a downstream crate can use.

  Scenario Outline: Compile and execute conditional functions
    Given Rust functions using let bindings and nested conditionals
    When I compile the functions to Morphir IR version "<version>"
    And I generate Rust from the compiled model
    Then the generated functions return the expected branch results

    Examples:
      | version |
      | 3       |
      | 4       |

  Scenario: Extract native and external declarations
    Given Rust functions annotated as native and external bindings
    When I compile the bindings to Morphir IR version 4
    Then the IR preserves both binding kinds and their signatures

  Scenario Outline: Compile and generate a product and a sum
    Given a Rust model with a product and a sum with payloads
    When I compile the model to Morphir IR version "<version>"
    And I generate Rust from the compiled model
    Then a Rust consumer can construct and match the generated types

    Examples:
      | version |
      | 3       |
      | 4       |

  Scenario Outline: Reject unsupported borrowed types without a partial model
    Given a Rust model containing a borrowed field
    When I compile the model to Morphir IR version "<version>"
    Then compilation fails without partial IR

    Examples:
      | version |
      | 3       |
      | 4       |
