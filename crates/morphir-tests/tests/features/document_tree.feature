Feature: Document Tree Loading
  I want to load Morphir IR from a directory structure (Document Tree)
  So that I can work with unbundled source files

  Scenario: Load V4 Package from Directory
    Given I have a Memory VFS
    And I have a project structure with the following files:
      | morphir.json          | {"name": "test-package"} |
      | src/Test/Package.json | {}                       |
      | src/Test/Module.json  | {}                       |
    When I load the distribution from the directory
    Then I should get a valid "v4" IR distribution
    And the package name should be "test-package"
