Feature: File and output steps
  Scenario: A written file reads back
    Given a file "a/b.txt" containing:
      """
      line one
      line two
      """
    Then the file "a/b.txt" should exist
    And the file "a/b.txt" should contain:
      """
      line one
      line two
      """

  Scenario: Output checks
    Given the output:
      """
      {"result": {"count": 2}}
      """
    Then stdout should contain "count"
    And the JSON output at "/result/count" should be:
      """
      2
      """
