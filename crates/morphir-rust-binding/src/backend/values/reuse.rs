//! Reuse immutable function handles without requiring Clone on arbitrary values.
use super::*;

pub(super) fn reusable(tpe: &Type) -> bool {
    crate::functions::copy_type(tpe)
        || matches!(tpe, Type::Function(..))
        || matches!(tpe,Type::Tuple(_,fields) if fields.iter().all(reusable))
}

/// The caller supplies a variable or field projection, never an effectful expression.
pub(super) fn value(expression: TokenStream, tpe: &Type) -> TokenStream {
    match tpe {
        Type::Function(..) => quote!(::std::rc::Rc::clone(&(#expression))),
        Type::Tuple(_, fields) if !crate::functions::copy_type(tpe) => {
            let fields = fields.iter().enumerate().map(|(index, tpe)| {
                let index = syn::Index::from(index);
                value(quote!((#expression).#index), tpe)
            });
            quote!((#(#fields,)*))
        }
        _ => expression,
    }
}
