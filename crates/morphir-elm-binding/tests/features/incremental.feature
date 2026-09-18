Feature: Incremental Elm compilation
  The host keeps the baseline, the extension decides per module whether to
  reuse it. Every scenario runs the same two-module package, where A imports B
  and aliases one of its types, and threads each run's module results back into
  the next run's baseline, exactly as a daemon would.

  Scenario: A changed dependent recompiles against a broken dependency baseline
    Given a baseline from compiling A "original" and B "original"
    When I compile A "changed" and B "broken"
    Then compilation fails
    And module "A" is compiled
    And module "B" is failed
    And module "B" reports diagnostic "ELM_SYNTAX"
    And module "B" reports no IR
    And the distribution contains modules "A"

  Scenario: Fixing a dependency leaves an untouched dependent unchanged
    Given a baseline from compiling A "original" and B "original"
    And a further baseline from compiling A "changed" and B "broken"
    When I compile A "changed" and B "documented"
    Then compilation succeeds
    And module "B" is compiled
    And module "A" is unchanged
    And the distribution contains modules "B, A"

  Scenario: Changing a dependency interface recompiles its dependents
    Given a baseline from compiling A "original" and B "original"
    When I compile A "original" and B "retyped"
    Then compilation succeeds
    And module "B" is compiled
    And module "B" has a new interface digest
    And module "A" is compiled

  Scenario: Widening a dependency interface recompiles its dependents
    Given a baseline from compiling A "original" and B "original"
    When I compile A "original" and B "widened"
    Then compilation succeeds
    And module "B" has a new interface digest
    And module "A" is compiled

  Scenario: A documentation-only change does not recompile dependents
    Given a baseline from compiling A "original" and B "original"
    When I compile A "original" and B "documented"
    Then compilation succeeds
    And module "B" is compiled
    And module "B" has a new source digest
    And module "B" keeps its interface digest
    And module "A" is unchanged

  Scenario: A broken dependency without a baseline blocks its dependents
    Given no baseline
    When I compile A "original" and B "broken"
    Then compilation fails
    And module "B" is failed
    And module "A" is blocked
    And module "A" reports diagnostic "ELM_BLOCKED"
    And module "A" carries a located diagnostic

  Scenario: A broken dependency with a baseline leaves dependents unchanged
    Given a baseline from compiling A "original" and B "original"
    When I compile A "original" and B "broken"
    Then compilation fails
    And module "B" is failed
    And module "A" is unchanged
    And the distribution contains modules "A"

  Scenario: A baseline that will not say which context it came from is ignored
    Given a baseline from compiling A "original" and B "original"
    And the baseline forgets which context it came from
    When I compile A "original" and B "original"
    Then compilation succeeds
    And the compile reports the baseline was ignored because it "carries no contextDigest"
    And module "A" is compiled
    And module "B" is compiled
    And the compile reports the context it ran under

  Scenario: Deleting a dependency fails its dependents
    Given a baseline from compiling A "original" and B "original"
    When I compile A "original" with B deleted
    Then compilation fails
    And the compile reports 1 module
    And module "A" is failed
    And module "A" reports diagnostic "ELM_RESOLVE_NOT_FOUND"
