# Feature: Extensions fill the context

`@probe:feature`

## Scenario: The feature tag reaches the scenario

* Then the probe value is "feature"

## Scenario: A scenario tag overrides it

`@probe:scenario`

* Then the probe value is "scenario"

## Scenario: A failing step reports the markdown line

* Then the probe value is "never"

## Scenario Outline: The feature tag reaches an outline row

* Then the probe value is "<value>"

### Examples: One row

| value    |
| -------- |
| feature  |
