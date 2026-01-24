// Adapted from morphir-examples/src/Morphir/Sample/Apps/Order/Order.elm
// Functions with pattern matching and business logic

pub type OrderPrice {
  Market
  Limit(Float)
}

pub type Violations {
  InvalidPrice(Float)
  InvalidQuantity(Int)
}

pub type BuyRequest {
  BuyRequest(
    id: String,
    request_price: OrderPrice,
    quantity: Int,
    product: String,
  )
}

pub fn validate(request: BuyRequest) -> List(Violations) {
  let price_check = case request.request_price {
    Market -> []
    Limit(p) -> case p < 0.0 {
      True -> [InvalidPrice(p)]
      False -> []
    }
  }
  
  let quantity_check = case request.quantity <= 0 {
    True -> [InvalidQuantity(request.quantity)]
    False -> []
  }
  
  price_check <> quantity_check
}

pub fn lockin_price(request_price: OrderPrice, market_price: Float) -> Float {
  case request_price {
    Market -> market_price
    Limit(p) -> p
  }
}
