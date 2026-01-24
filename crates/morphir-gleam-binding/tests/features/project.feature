Feature: Parse Gleam Project
  As a developer
  I want to parse entire Gleam projects
  So that I can handle multi-module codebases

  @parse-project
  Scenario Outline: Parse project with multiple modules
    Given I have a Gleam project at "<project_path>"
    And the project has the following structure:
      | path                    | content                         |
      | src/main.gleam         | pub fn main() { "hello" }       |
      | src/utils/helper.gleam | pub fn helper() { 42 }           |
    When I parse the project
    Then parsing should succeed
    And I should get <module_count> modules
    And module "<module_name>" should exist
    And module "<module_name>" should have <value_count> values

    Examples:
      | project_path    | module_count | module_name      | value_count |
      | minimal_project | 1            | main             | 1           |
      | multi_module    | 2            | main             | 1           |
      | multi_module    | 2            | utils/helper     | 1           |

  @parse-project @real-world
  Scenario: Parse real-world shared types project
    Given I have a Gleam project at "shared_types"
    And the project has the following structure:
      | path                    | content                                                      |
      | src/client.gleam       | pub type ID { ID(String) }                                  |
      | src/product.gleam     | pub type ID { ID(String) }                                   |
      | src/price.gleam        | pub type Price { Price(Float) }                             |
      | src/quantity.gleam     | pub type Quantity { Quantity(Int) }                        |
    When I parse the project
    Then parsing should succeed
    And I should get 4 modules
    And module "client" should exist
    And module "product" should exist
    And module "price" should exist
    And module "quantity" should exist

  @parse-project @real-world
  Scenario: Parse real-world order types project
    Given I have a Gleam project at "order_types"
    And the project has the following structure:
      | path                    | content                                                      |
      | src/order_price.gleam  | pub type OrderPrice { Market Limit(Float) }                 |
      | src/violations.gleam   | pub type Violations { InvalidPrice(Float) InvalidQuantity(Int) } |
      | src/reject_reason.gleam| pub type RejectReason { InsufficientInventory DisagreeablePrice } |
    When I parse the project
    Then parsing should succeed
    And I should get 3 modules
    And module "order_price" should exist
    And module "violations" should exist
    And module "reject_reason" should exist
