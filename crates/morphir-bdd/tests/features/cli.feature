# Quoting: `When I run {string}` splits its argument like a shell. Inside a step text written
# with double quotes, `\"` groups words (it does not embed a literal `"`, since cucumber's
# `{string}` capture is not unescaped). To pass an argument that itself contains a literal `"`,
# write the step's `{string}` with single quotes instead, for example
# `When I run 'morphir x "a b"'`; the double quotes inside it group as usual and arrive intact.
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

  Scenario: A single-quoted step text groups a literal double-quoted argument
    When I run 'morphir echo "a file.json"'
    Then the command should succeed
    And stdout should contain "a file.json"
