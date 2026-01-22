Feature: Virtual File System Globbing

  Scenario: Globbing files in a memory VFS
    Given I have a Memory VFS
    And I create a file "foo.txt"
    And I create a file "bar.rs"
    And I create a file "baz/qux.txt"
    When I glob for "**/*.txt"
    Then I should find "foo.txt"
    And I should find "baz/qux.txt"
    And I should not find "bar.rs"

  Scenario Outline: Globbing in a complex project structure
    Given I have a Memory VFS
    And I have a project structure with the following files:
      | src/main.rs          |
      | src/lib.rs           |
      | src/utils/mod.rs     |
      | src/utils/helper.rs  |
      | tests/integration.rs |
      | README.md            |
      | target/debug/app     |
    When I glob for "<pattern>"
    Then I should find "<match>"
    And I should not find "<no_match>"

    Examples:
      | pattern     | match               | no_match             |
      | **/*.rs     | src/main.rs         | README.md            |
      | src/**/*.rs | src/utils/helper.rs | tests/integration.rs |
      | **/mod.rs   | src/utils/mod.rs    | src/lib.rs           |
      | target/**/* | target/debug/app    | src/main.rs          |
