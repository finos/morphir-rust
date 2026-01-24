// Adapted from morphir-examples/src/Morphir/Sample/Apps/Order/Order.elm
// Simple union type with multiple variants (no associated data)

pub type RejectReason {
  InsufficientInventory
  DisagreeablePrice
}
