// Adapted from morphir-examples/tutorial/step_1_first_logic/src/Morphir/Example/App/Rentals.elm
// Simple function with Result type

pub fn request(availability: Int, requested_quantity: Int) -> Result(Int, String) {
  case requested_quantity <= availability {
    True -> Ok(requested_quantity)
    False -> Error("Insufficient availability")
  }
}
