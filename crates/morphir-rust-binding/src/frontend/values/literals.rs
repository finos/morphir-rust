use super::{Lower, Typed, scalar};
use crate::Outcome;
use morphir_core::ir::classic::{Literal, Value};

impl Lower<'_, '_> {
    pub(super) fn literal(&self, lit: &syn::Lit, negative: bool) -> Outcome<Typed> {
        let (literal, ty) = match lit {
            syn::Lit::Bool(b) if !negative => (Literal::Bool(b.value), scalar("Basics", "Bool")),
            syn::Lit::Char(c) if !negative => (Literal::Char(c.value()), scalar("Char", "Char")),
            syn::Lit::Int(i) if i.suffix() == "f64" => {
                let value = i.base10_parse::<f64>().map_err(|_| self.error(lit, "Invalid float literal"))?;
                if !value.is_finite() { return Err(self.error(lit, "Float literals must be finite")); }
                (Literal::Float(if negative { -value } else { value }), scalar("Basics", "Float"))
            }
            syn::Lit::Int(i) if i.suffix().is_empty() || i.suffix() == "i64" => {
                let value = i.base10_parse::<i128>().map_err(|_| self.error(lit, "Integer literal is outside i64 range"))?;
                let value = if negative { -value } else { value };
                let value = i64::try_from(value).map_err(|_| self.error(lit, "Integer literal is outside i64 range"))?;
                (Literal::WholeNumber(value), scalar("Basics", "Int"))
            }
            syn::Lit::Float(f) if f.suffix().is_empty() || f.suffix() == "f64" => {
                let value = f.base10_parse::<f64>().map_err(|_| self.error(lit, "Invalid float literal"))?;
                if !value.is_finite() { return Err(self.error(lit, "Float literals must be finite")); }
                (Literal::Float(if negative { -value } else { value }), scalar("Basics", "Float"))
            }
            _ => return Err(self.error(lit, "Only i64, f64, bool and char literals are supported; borrowed strings are not owned String values")),
        };
        Ok((Value::Literal(ty.clone(), literal), ty))
    }
}
