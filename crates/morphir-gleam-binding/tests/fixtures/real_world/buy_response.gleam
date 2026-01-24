// Adapted from morphir-examples/src/Morphir/Sample/Apps/Order/Order.elm
// Complex union type with multiple variants, each with different associated data

pub type BuyResponse {
  BuyAccepted(String, String, Float, Int)
  BuyInvalid(String, List(Violations))
  BuyRejected(String, List(RejectReason))
}

pub type Violations {
  InvalidPrice(Float)
  InvalidQuantity(Int)
}

pub type RejectReason {
  InsufficientInventory
  DisagreeablePrice
}
