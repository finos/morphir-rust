use crate::{Outcome, error};
use morphir_core::ir::classic::{Access, Name};
use morphir_extension_sdk::{
    Diagnostic, SourceDocument, SourceLocation, SourcePosition, SourceRange,
};
use proc_macro2::Span;
use syn::{Token, punctuated::Punctuated, spanned::Spanned};

pub(super) struct Source<'a>(pub &'a SourceDocument);
impl Source<'_> {
    pub fn error(&self, span: Span, code: &str, message: impl Into<String>) -> Diagnostic {
        let position = |point: proc_macro2::LineColumn| {
            let prefix: String = self
                .0
                .text
                .lines()
                .nth(point.line.saturating_sub(1))
                .unwrap_or("")
                .chars()
                .take(point.column)
                .collect();
            SourcePosition::from_line_prefix(point.line.saturating_sub(1) as u32, &prefix)
        };
        Diagnostic {
            location: Some(SourceLocation {
                uri: self.0.uri.clone(),
                range: SourceRange {
                    start: position(span.start()),
                    end: position(span.end()),
                },
            }),
            ..error(code, message)
        }
    }
    pub fn name(&self, ident: &syn::Ident) -> Outcome<Name> {
        let spelling = ident.to_string();
        let spelling = spelling.strip_prefix("r#").unwrap_or(&spelling);
        if !spelling.is_ascii() || spelling.chars().all(|c| c == '_') {
            return Err(self.error(
                ident.span(),
                "RS_NAME",
                "Names must contain ASCII letters or digits",
            ));
        }
        Ok(spelling.parse().expect("infallible classic name"))
    }
    pub fn visibility(&self, visibility: &syn::Visibility) -> Outcome<Access> {
        match visibility {
            syn::Visibility::Public(_) => Ok(Access::Public),
            syn::Visibility::Inherited => Ok(Access::Private),
            _ => Err(self.error(
                visibility.span(),
                "RS_VISIBILITY",
                "Restricted visibility is not supported",
            )),
        }
    }
    pub fn attributes(&self, attrs: &[syn::Attribute], derives: bool) -> Outcome<String> {
        let mut docs = vec![];
        for attr in attrs {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(meta) = &attr.meta
                    && let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    }) = &meta.value
                {
                    docs.push(text.value().trim().to_owned());
                    continue;
                }
            } else if derives && attr.path().is_ident("derive") {
                let paths = attr
                    .parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
                    .map_err(|e| self.error(e.span(), "RS_ATTRIBUTE", e.to_string()))?;
                if !paths.is_empty()
                    && paths.iter().all(|p| {
                        [
                            "Debug",
                            "Clone",
                            "Copy",
                            "PartialEq",
                            "Eq",
                            "PartialOrd",
                            "Ord",
                            "Hash",
                            "Default",
                        ]
                        .iter()
                        .any(|n| p.is_ident(n))
                    })
                {
                    continue;
                }
            }
            return Err(self.error(
                attr.span(),
                "RS_ATTRIBUTE",
                "Only documentation and ordinary built-in derives are supported",
            ));
        }
        Ok(docs.join("\n"))
    }
    pub fn unique(&self, idents: impl IntoIterator<Item = syn::Ident>) -> Outcome<Vec<Name>> {
        let mut names = Vec::new();
        for ident in idents {
            let name = self.name(&ident)?;
            if names.contains(&name) {
                return Err(self.error(
                    ident.span(),
                    "RS_DUPLICATE_NAME",
                    "Duplicate name or Morphir name normalization collision",
                ));
            }
            names.push(name);
        }
        Ok(names)
    }
    pub fn generics(&self, generics: &syn::Generics) -> Outcome<Vec<syn::Ident>> {
        if let Some(clause) = &generics.where_clause {
            return Err(self.error(
                clause.span(),
                "RS_GENERICS",
                "Where clauses are unsupported",
            ));
        }
        let params = generics
            .params
            .iter()
            .map(|p| match p {
                syn::GenericParam::Type(t) if t.bounds.is_empty() && t.default.is_none() => {
                    self.attributes(&t.attrs, false)?;
                    Ok(t.ident.clone())
                }
                _ => Err(self.error(
                    p.span(),
                    "RS_GENERICS",
                    "Only unconstrained type parameters are supported",
                )),
            })
            .collect::<Outcome<Vec<_>>>()?;
        self.unique(params.clone())?;
        Ok(params)
    }
}
