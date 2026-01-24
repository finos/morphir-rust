// Adapted from morphir-examples/src/Morphir/Sample/Apps/Order/Order.elm
// Union type with multiple variants, each with different associated data

pub type Violations {
  InvalidPrice(Float)
  InvalidQuantity(Int)
}
