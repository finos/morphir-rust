@node:Value @version:4
Feature: Indented content
  The feature text.

  ```yaml morphir
  outer:
    inner: 1

    after_blank: 2
  ```

  More prose.

  @spelling
  Scenario: values-0003 Reference shorthand
    Then its canonical Ion spelling is:
      """ion
      (ref 'morphir/SDK:basics#add')
        (deeper)
      """
    And its canonical YAML spelling is:
      ```yaml
      Reference:
        name: morphir/SDK:basics#add
      ```
