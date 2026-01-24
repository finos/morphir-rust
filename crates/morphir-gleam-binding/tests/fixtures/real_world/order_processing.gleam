// Adapted from morphir-examples/src/Morphir/Sample/Apps/Order/Order.elm
// Complete order processing example with business logic

pub type OrderPrice {
  Market
  Limit(Float)
}

pub type Violations {
  InvalidPrice(Float)
  InvalidQuantity(Int)
}

pub type RejectReason {
  InsufficientInventory
  DisagreeablePrice
}

pub type BuyResponse {
  BuyAccepted(String, String, Float, Int)
  BuyInvalid(String, List(Violations))
  BuyRejected(String, List(RejectReason))
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

pub fn process_buy(
  request: BuyRequest,
  market_price: Float,
  available_inventory: Int,
) -> BuyResponse {
  let violations = validate(request)
  let lock_price = lockin_price(request.request_price, market_price)
  
  case violations {
    [] -> {
      let price_check = case lock_price < market_price * 0.9 {
        True -> [DisagreeablePrice]
        False -> []
      }
      
      let availability_check = case available_inventory < request.quantity {
        True -> [InsufficientInventory]
        False -> []
      }
      
      let rejections = price_check <> availability_check
      
      case rejections {
        [] -> BuyAccepted(request.id, request.product, lock_price, request.quantity)
        _ -> BuyRejected(request.id, rejections)
      }
    }
    _ -> BuyInvalid(request.id, violations)
  }
}
