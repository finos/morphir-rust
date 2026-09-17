//! Explicit binding metadata is parsed as data, without expanding Rust macros.
use super::source::Source;
use crate::Outcome;
use morphir_core::ir::v4::value::{ExternalBinding, NativeHint, NativeInfo, ValueBody};
use std::collections::BTreeMap;
use syn::{Token, punctuated::Punctuated, spanned::Spanned};

pub(super) enum Binding {
    Native(NativeInfo),
    External(Vec<ExternalBinding>),
}

impl Binding {
    pub fn into_body(self) -> ValueBody {
        match self {
            Self::Native(native_info) => ValueBody::Native { native_info },
            Self::External(externals) => ValueBody::External {
                externals,
                fallback: None,
            },
        }
    }
}

pub(super) struct Metadata {
    pub documentation: String,
    pub binding: Option<Binding>,
}

enum Kind {
    Native,
    External,
}

fn kind(attr: &syn::Attribute) -> Option<Kind> {
    let path = attr.path();
    if path.leading_colon.is_some()
        || path.segments.len() != 2
        || path.segments[0].ident != "morphir"
        || path
            .segments
            .iter()
            .any(|s| !matches!(s.arguments, syn::PathArguments::None))
    {
        return None;
    }
    match path.segments[1].ident.to_string().as_str() {
        "native" => Some(Kind::Native),
        "external" => Some(Kind::External),
        _ => None,
    }
}

pub(super) fn is_binding_attribute(attr: &syn::Attribute) -> bool {
    kind(attr).is_some()
}

pub(super) fn parse(source: &Source<'_>, attrs: &[syn::Attribute]) -> Outcome<Metadata> {
    let mut ordinary = Vec::new();
    let mut binding = None;
    for attr in attrs {
        match kind(attr) {
            Some(Kind::Native) => {
                if binding.is_some() {
                    return Err(source.error(attr.span(), "RS_BINDING_ATTRIBUTE", "A native binding must be unique and cannot be mixed with external bindings"));
                }
                binding = Some(Binding::Native(native(source, attr)?));
            }
            Some(Kind::External) => {
                let external = external(source, attr)?;
                match &mut binding {
                    None => binding = Some(Binding::External(vec![external])),
                    Some(Binding::External(externals)) => {
                        if externals
                            .iter()
                            .any(|e| e.target_platform == external.target_platform)
                        {
                            return Err(source.error(
                                attr.span(),
                                "RS_BINDING_ATTRIBUTE",
                                "External target platforms must be unique",
                            ));
                        }
                        externals.push(external);
                    }
                    Some(Binding::Native(_)) => {
                        return Err(source.error(
                            attr.span(),
                            "RS_BINDING_ATTRIBUTE",
                            "Native and external bindings cannot be mixed",
                        ));
                    }
                }
            }
            _ => ordinary.push(attr.clone()),
        }
    }
    Ok(Metadata {
        documentation: source.attributes(&ordinary, false)?,
        binding,
    })
}

fn fields(
    source: &Source<'_>,
    attr: &syn::Attribute,
    allowed: &[&str],
) -> Outcome<BTreeMap<String, String>> {
    let entries = attr
        .parse_args_with(Punctuated::<syn::MetaNameValue, Token![,]>::parse_terminated)
        .map_err(|e| source.error(e.span(), "RS_BINDING_ATTRIBUTE", e.to_string()))?;
    let mut result = BTreeMap::new();
    for entry in entries {
        let Some(key) = entry
            .path
            .get_ident()
            .map(ToString::to_string)
            .filter(|key| allowed.contains(&key.as_str()))
        else {
            return Err(source.error(
                entry.path.span(),
                "RS_BINDING_ATTRIBUTE",
                "Unknown binding attribute key",
            ));
        };
        let value = match &entry.value {
            syn::Expr::Lit(syn::ExprLit {
                attrs,
                lit: syn::Lit::Str(text),
            }) if attrs.is_empty() => text.value(),
            _ => {
                return Err(source.error(
                    entry.value.span(),
                    "RS_BINDING_ATTRIBUTE",
                    "Binding attribute values must be string literals",
                ));
            }
        };
        if result.insert(key, value).is_some() {
            return Err(source.error(
                entry.span(),
                "RS_BINDING_ATTRIBUTE",
                "Binding attribute keys must be unique",
            ));
        }
    }
    Ok(result)
}

fn required(
    source: &Source<'_>,
    attr: &syn::Attribute,
    fields: &BTreeMap<String, String>,
    key: &str,
) -> Outcome<String> {
    fields
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| {
            source.error(
                attr.span(),
                "RS_BINDING_ATTRIBUTE",
                format!("Binding attribute requires a nonblank {key}"),
            )
        })
}

fn native(source: &Source<'_>, attr: &syn::Attribute) -> Outcome<NativeInfo> {
    let fields = fields(source, attr, &["hint", "platform", "description"])?;
    let hint = required(source, attr, &fields, "hint")?;
    if hint != "platform_specific" && fields.contains_key("platform") {
        return Err(source.error(
            attr.span(),
            "RS_BINDING_ATTRIBUTE",
            "Only platform_specific native hints accept platform",
        ));
    }
    let hint = match hint.as_str() {
        "arithmetic" => NativeHint::Arithmetic,
        "comparison" => NativeHint::Comparison,
        "string_op" => NativeHint::StringOp,
        "collection_op" => NativeHint::CollectionOp,
        "platform_specific" => NativeHint::PlatformSpecific {
            platform: required(source, attr, &fields, "platform")?,
        },
        _ => {
            return Err(source.error(
                attr.span(),
                "RS_BINDING_ATTRIBUTE",
                "Unsupported native hint",
            ));
        }
    };
    Ok(NativeInfo {
        hint,
        description: fields.get("description").cloned(),
    })
}

fn external(source: &Source<'_>, attr: &syn::Attribute) -> Outcome<ExternalBinding> {
    let fields = fields(source, attr, &["target", "name"])?;
    Ok(ExternalBinding {
        target_platform: required(source, attr, &fields, "target")?,
        external_name: required(source, attr, &fields, "name")?,
    })
}
