Feature: Parse Gleam Project
  As a developer
  I want to parse entire Gleam projects
  So that I can handle multi-module codebases

  @parse-project
  Scenario Outline: Parse project with multiple modules
    Given I have a Gleam project at "<project_path>"
    And the project has the following structure:
      | path                    | content                         |
      | src/main.gleam         | pub fn main() { "hello" }       |
      | src/utils/helper.gleam | pub fn helper() { 42 }           |
    When I parse the project
    Then parsing should succeed
    And I should get <module_count> modules
    And module "<module_name>" should exist
    And module "<module_name>" should have <value_count> values

    Examples:
      | project_path    | module_count | module_name      | value_count |
      | minimal_project | 1            | main             | 1           |
      | multi_module    | 2            | main             | 1           |
      | multi_module    | 2            | utils/helper     | 1           |
