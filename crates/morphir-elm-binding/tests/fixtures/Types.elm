module My.Domain.Types exposing (Account, Status(..), Id)

{-| Module docs. -}

import Dict exposing (Dict)
import My.Other as Other


{-| An account. -}
type alias Account a =
    { id : Id
    , tags : List String
    , extra : Dict String (Maybe a)
    , pair : ( Int, Other.Thing )
    , f : Int -> { r | name : String } -> ()
    }


type Status
    = Active
    | Closed String Int
    | Pending { reason : String }


type alias Id =
    String


greet : String -> String
greet name =
    "hi " ++ name
