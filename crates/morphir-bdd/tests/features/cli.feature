Feature: CLI process steps
  Scenario: Arguments and an isolated home
    When I run "morphir ir migrate \"a file.json\""
    Then the command should succeed
    And stdout should contain "a file.json"
    And stdout should contain "home="

  Scenario: A failing command
    When I run "morphir fail"
    Then the command should fail
    And the exit code should be 3
