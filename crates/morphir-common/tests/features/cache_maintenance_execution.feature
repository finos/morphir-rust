Feature: Bounded cache cleanup execution
  As a Morphir user
  I want cleanup to revalidate ownership and recover interrupted work
  So that manual and automatic maintenance remain safe and bounded

  Scenario: Remove only planner-selected owned content
    Given an owned cleanup candidate and an unknown cache entry
    When I execute the cleanup plan
    Then the owned candidate is removed
    And the unknown cache entry remains

  Scenario: Honor a lease acquired after planning
    Given a cleanup candidate that acquires a lease after planning
    When I execute the cleanup plan
    Then the late lease defers removal
    And the leased cache entry remains

  Scenario: Stop at the per-run removal budget
    Given two planner-selected cleanup candidates and a one-removal budget
    When I execute the cleanup plan
    Then one candidate is removed and one is deferred

  Scenario: Recover an interrupted trash run
    Given content left by an interrupted cleanup trash run
    When I execute the cleanup plan
    Then the interrupted trash run is removed
