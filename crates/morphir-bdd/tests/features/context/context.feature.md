# Feature: Extensions fill the context

`@probe:feature`

## Scenario: The feature tag reaches the scenario

* Then the probe value is "feature"

## Scenario: A scenario tag overrides it

`@probe:scenario`

* Then the probe value is "scenario"

## Scenario: A failing step reports the markdown line

* Then the probe value is "never"

## Scenario Outline: The <source> tag reaches an outline row

* Then the probe value is "<value>"

### Examples: First rows

`@probe:first-block`

| source | value       |
| ------ | ----------- |
| first  | first-block |
| again  | first-block |

### Examples: Second rows

`@probe:second-block`

| source | value        |
| ------ | ------------ |
| second | second-block |

### Examples: Untagged rows

| source   | value   |
| -------- | ------- |
| untagged | feature |
