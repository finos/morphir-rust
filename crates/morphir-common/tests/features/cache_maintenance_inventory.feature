Feature: Safe cache namespace inventory
  As a Morphir component author
  I want cache inventory to honor ownership and filesystem safety
  So that cleanup removes only bounded, positively owned content

  Scenario: Classify owned, leased, and unknown entries
    Given a cache namespace with disposable, leased, and unknown entries
    When I inventory the cache namespace
    Then the disposable entry is measured as removable ownership
    And the leased entry remains protected
    And the unknown entry remains unclassified

  Scenario: Stop when the inventory entry budget is exhausted
    Given a cache namespace that exceeds a one-entry inventory budget
    When I inventory the cache namespace with a one-entry budget
    Then inventory fails closed with an entry-limit diagnostic

  Scenario: Refuse a link-like namespace root
    Given a cache namespace root that links outside Morphir Home
    When I inventory the cache namespace
    Then inventory refuses the link-like namespace root
    And content outside Morphir Home remains unchanged
