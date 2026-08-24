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

  Scenario: Load YAML Project Configuration
    Given I have a "morphir.yaml" file with:
      """
      project:
        name: My.Project
        version: "1.0.0"
        source_directory: src
        exposed_modules:
          - App
      """
    When I load the configuration
    Then it should be a project configuration
    And the project name should be "My.Project"
    And the source directory should be "src"

  Scenario: Later configuration sources take precedence
    Given a base configuration value:
      """
      {"ir": {"format_version": 3, "strict_mode": false}, "logging": {"level": "info"}}
      """
    And an overlay configuration value:
      """
      {"ir": {"strict_mode": true}, "ui": {"theme": "dark"}}
      """
    When I merge the configuration values
    Then the merged value at "ir.format_version" should be 3
    And the merged value at "ir.strict_mode" should be true
    And the merged value at "logging.level" should be "info"
    And the merged value at "ui.theme" should be "dark"

  Scenario: Arrays replace instead of concatenating
    Given a base configuration value:
      """
      {"codegen": {"targets": ["go", "scala"]}}
      """
    And an overlay configuration value:
      """
      {"codegen": {"targets": ["typescript"]}}
      """
    When I merge the configuration values
    Then the merged value at "codegen.targets" should be ["typescript"]

  Scenario: Null overlay values do not override
    Given a base configuration value:
      """
      {"frontend": {"language": "elm"}, "cache": {"enabled": true}}
      """
    And an overlay configuration value:
      """
      {"frontend": null, "cache": {"enabled": null, "dir": ".cache"}, "logging": null}
      """
    When I merge the configuration values
    Then the merged value at "frontend.language" should be "elm"
    And the merged value at "cache.enabled" should be true
    And the merged value at "cache.dir" should be ".cache"
    And the merged value should not contain "logging"
    And the base configuration value should be unchanged:
      """
      {"frontend": {"language": "elm"}, "cache": {"enabled": true}}
      """

  Scenario: Environment variables map to nested configuration keys
    Given the environment variable "MORPHIR_IR__STRICT_MODE" is "true"
    And the environment variable "MORPHIR_IR__FORMAT_VERSION" is "4"
    And the environment variable "MORPHIR_CODEGEN__TARGETS" is '["go", "typescript"]'
    And the environment variable "MORPHIR_LOGGING__LEVEL" is "debug"
    And the environment variable "MORPHIR_IR_MODE" is "vfs"
    And the environment variable "HOME" is "/home/alice"
    When I load the environment configuration
    Then the merged value at "ir.strict_mode" should be true
    And the merged value at "ir.format_version" should be 4
    And the merged value at "codegen.targets" should be ["go", "typescript"]
    And the merged value at "logging.level" should be "debug"
    And the merged value at "ir_mode" should be "vfs"
    And the merged value should not contain "home"
