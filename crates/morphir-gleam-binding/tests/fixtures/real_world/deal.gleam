// Adapted from morphir-examples/src/Morphir/Sample/Apps/Order/Order.elm
// Record type example

pub type Deal {
  Deal(id: String, product: String, price: Float, quantity: Int)
}
