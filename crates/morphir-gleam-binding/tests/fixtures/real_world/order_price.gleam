// Adapted from morphir-examples/src/Morphir/Sample/Apps/Order/Order.elm
// Union type with variants (one with no data, one with data)

pub type OrderPrice {
  Market
  Limit(Float)
}
