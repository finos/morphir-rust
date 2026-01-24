# WIP: Workspace parsing not yet implemented
# See: https://github.com/finos/morphir-rust/issues/40
@wip
Feature: Parse Gleam Workspace
  As a developer
  I want to parse Gleam workspaces with multiple projects
  So that I can handle complex multi-project codebases

  @parse-workspace
  Scenario Outline: Parse workspace with multiple projects
    Given I have a Gleam workspace at "<workspace_path>"
    And the workspace has the following structure:
      | path                      | content                    |
      | project_a/src/main.gleam | pub fn main() { "a" }      |
      | project_b/src/main.gleam | pub fn main() { "b" }      |
    When I parse the workspace
    Then parsing should succeed
    And I should get <project_count> projects
    And project "<project_name>" should have <module_count> modules

    Examples:
      | workspace_path   | project_count | project_name | module_count |
      | example_workspace | 2             | project_a    | 1            |
      | example_workspace | 2             | project_b    | 1            |
