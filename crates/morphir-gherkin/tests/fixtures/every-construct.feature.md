# Feature: Every construct

`@feature-tag`

Feature description with a [link](kb/x.md).

```yaml morphir
outer:
  inner: 1
```

## Background: Shared setup

* Given a background step

## Rule: The first rule

Rule description.

### Scenario: Plain scenario

`@scenario-tag`

* Given a step with a table

  | a | b |
  | - | - |
  | 1 | 2 |

* When a step with a doc string

  ```ion
  (ref 'morphir/SDK:basics#add')
  ```

* Then a result

### Scenario Outline: An outline

* Then <x> is <y>

#### Examples: Some rows

`@examples-tag`

| x | y |
| - | - |
| 1 | 2 |
