Feature: Configuration Loading
  I want to load Morphir configuration from files
  So that I can customize the tool's behavior

  Scenario: Load Workspace Configuration
    Given I have a "morphir.toml" file with:
      """
      [workspace]
      members = ["a", "b"]
      """
    When I load the configuration
    Then it should be a workspace configuration
    And the workspace should have 2 members

  Scenario: Load Project Configuration
    Given I have a "morphir.toml" file with:
      """
      [project]
      name = "My.Project"
      version = "1.0.0"
      source_directory = "src"
      exposed_modules = []
      """
    When I load the configuration
    Then it should be a project configuration
    And the project name should be "My.Project"

  Scenario: Load Legacy Project Configuration
    Given I have a "morphir.json" file with:
      """
      {
          "name": "Legacy.Project",
          "sourceDirectory": "src",
          "exposedModules": ["App"]
      }
      """
    When I load the configuration
    Then it should be a project configuration
    And the project name should be "Legacy.Project"
    And the source directory should be "src"
