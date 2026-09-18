mod declarations;
mod ir;
mod names;
mod types;
mod values;

use crate::{Outcome, error};
use ir::{Body, Package};
use morphir_core::{
    ir::v4::{Access, Type},
    naming::FQName,
};
use morphir_extension_sdk::{Artifact, DiagnosticSeverity, GenerateRequest, GenerateResult};
use names::*;
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
use types::{Renderer, Symbol, references};

pub(crate) fn generate(request: &GenerateRequest) -> Outcome<GenerateResult> {
    if request.target != "rust" {
        return Err(error("RS_TARGET", "Expected target rust"));
    }
    let external = external_types(request)?;
    let package = ir::decode(&request.ir)?;
    check_alias_cycles(&package)?;
    let mut renderer = Renderer {
        package: &package,
        symbols: BTreeMap::new(),
        external,
        names: BTreeMap::new(),
        helpers: BTreeMap::new(),
    };
    let mut root = Module::default();
    let mut module_paths = BTreeMap::new();
    for (module, access) in &package.modules {
        let path = module_names(module)?;
        let key = path.iter().map(ToString::to_string).collect::<Vec<_>>();
        if module_paths.insert(key, module).is_some() {
            return Err(error("RS_NAME", format!("Module name collision: {module}")));
        }
        root.ensure(&path, *access);
    }
    for declaration in &package.declarations {
        let name = type_name(&declaration.name)?;
        reserve(
            renderer
                .names
                .entry(declaration.module.clone())
                .or_default(),
            &name,
        )?;
        let module = module_names(&declaration.module)?;
        renderer.symbols.insert(
            declaration.fqname.clone(),
            Symbol {
                path: quote!(crate::#(#module::)* #name),
                arity: declaration.params.len(),
            },
        );
    }
    for function in &package.functions {
        let name = field_name(&function.owner.name)?;
        reserve(
            renderer
                .names
                .entry(function.owner.module.clone())
                .or_default(),
            &name,
        )?;
    }
    for module in package.modules.keys() {
        if module.is_empty() {
            continue;
        }
        let parts: Vec<_> = module.split('/').collect();
        for i in 0..parts.len() {
            let parent = parts[..i].join("/");
            let child = module_names(&parts[..=i].join("/"))?
                .pop()
                .expect("nonempty path");
            if renderer
                .names
                .get(&parent)
                .is_some_and(|names| names.contains(&child.to_string()))
            {
                return Err(error(
                    "RS_NAME",
                    format!("Module/type name collision: {module}"),
                ));
            }
        }
    }
    for declaration in &package.declarations {
        let tokens = declarations::render(&mut renderer, declaration)?;
        root.ensure(&module_names(&declaration.module)?, Access::Public)
            .items
            .push(tokens);
    }
    let context = values::semantic_context(&renderer)?;
    for function in &package.functions {
        let tokens = values::render(&mut renderer, function, &context)?;
        root.ensure(&module_names(&function.owner.module)?, Access::Public)
            .items
            .push(tokens);
    }
    for (module, helpers) in renderer.helpers {
        root.ensure(&module_names(&module)?, Access::Public)
            .items
            .extend(helpers);
    }
    let body = root.tokens();
    let file =
        syn::parse2::<syn::File>(quote!(#body)).map_err(|e| error("RS_RENDER", e.to_string()))?;
    let diagnostics = if package.omitted_values > 0 {
        let mut diagnostic = error(
            "RS_VALUES_OMITTED",
            format!(
                "Generation omitted {} non-expression value declarations",
                package.omitted_values
            ),
        );
        diagnostic.severity = DiagnosticSeverity::Warning;
        vec![diagnostic]
    } else {
        vec![]
    };
    Ok(GenerateResult {
        success: true,
        diagnostics,
        artifacts: vec![Artifact {
            path: "lib.rs".into(),
            content: prettyplease::unparse(&file),
            binary: false,
        }],
    })
}

fn external_types(request: &GenerateRequest) -> Outcome<BTreeMap<String, syn::Path>> {
    if request.options.keys().any(|key| key != "externalTypes") {
        return Err(error(
            "RS_OPTIONS",
            "Only externalTypes is a supported Rust backend option",
        ));
    }
    let Some(value) = request.options.get("externalTypes") else {
        return Ok(BTreeMap::new());
    };
    let object = value.as_object().ok_or_else(|| {
        error(
            "RS_OPTIONS",
            "externalTypes must map Morphir FQNames to Rust paths",
        )
    })?;
    object
        .iter()
        .map(|(key, value)| {
            let name: FQName = serde_json::from_value(key.clone().into())
                .map_err(|e| error("RS_OPTIONS", e.to_string()))?;
            if name.to_canonical_string() != *key {
                return Err(error("RS_OPTIONS", format!("Noncanonical FQName {key}")));
            }
            let path = syn::parse_str::<syn::Path>(value.as_str().ok_or_else(|| {
                error(
                    "RS_OPTIONS",
                    "An external type binding must be a Rust path string",
                )
            })?)
            .map_err(|e| error("RS_OPTIONS", e.to_string()))?;
            if path
                .segments
                .iter()
                .any(|s| !matches!(s.arguments, syn::PathArguments::None))
            {
                return Err(error(
                    "RS_OPTIONS",
                    "External Rust paths must not contain generic arguments",
                ));
            }
            Ok((key.clone(), path))
        })
        .collect()
}

fn check_alias_cycles(package: &Package) -> Outcome<()> {
    let graph: BTreeMap<_, _> = package
        .declarations
        .iter()
        .filter_map(|d| {
            let Body::Alias(t) = &d.body else {
                return None;
            };
            if matches!(t, Type::Record(..) | Type::ExtensibleRecord(..)) {
                return None;
            }
            let mut edges = BTreeSet::new();
            references(t, &mut edges, true);
            Some((d.fqname.clone(), edges))
        })
        .collect();
    fn visit(
        node: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> bool {
        if done.contains(node) {
            return false;
        }
        let Some(edges) = graph.get(node) else {
            return false;
        };
        if !active.insert(node.to_owned()) {
            return true;
        }
        if edges.iter().any(|next| visit(next, graph, active, done)) {
            return true;
        }
        active.remove(node);
        done.insert(node.to_owned());
        false
    }
    let mut done = BTreeSet::new();
    for node in graph.keys() {
        if visit(node, &graph, &mut BTreeSet::new(), &mut done) {
            return Err(error(
                "RS_TYPE",
                format!("Recursive type alias {node} requires a nominal struct or enum"),
            ));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Module {
    access: Option<Access>,
    items: Vec<TokenStream>,
    children: BTreeMap<String, Module>,
}

impl Module {
    fn ensure(&mut self, path: &[proc_macro2::Ident], access: Access) -> &mut Self {
        let Some((first, rest)) = path.split_first() else {
            self.access.get_or_insert(access);
            return self;
        };
        self.children
            .entry(first.to_string())
            .or_default()
            .ensure(rest, access)
    }

    fn tokens(&self) -> TokenStream {
        let items = &self.items;
        let children = self.children.iter().map(|(name, module)| {
            let name = syn::parse_str::<proc_macro2::Ident>(name).expect("validated identifier");
            let visibility = declarations::visibility(module.access.unwrap_or(Access::Public));
            let body = module.tokens();
            quote!(#visibility mod #name { #body })
        });
        quote!(#(#items)* #(#children)*)
    }
}
