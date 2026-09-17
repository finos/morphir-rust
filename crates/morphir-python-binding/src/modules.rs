//! Module identity and package-wide type information shared by both directions.

use crate::{Outcome, error, names, values::TupleAliases};
use morphir_core::ir::v4::*;

mod source_path;

/// A Python dotted import path and its validated Morphir module name.
pub(crate) struct ModuleIdentity {
    pub python: String,
    pub canonical: String,
}

impl ModuleIdentity {
    pub fn from_source(uri: &str, root: Option<&str>) -> Outcome<Self> {
        let uri = source_path::normalize(uri)?;
        let relative = if uri.starts_with('/') || uri.contains(':') {
            match root {
                Some(root) => {
                    let root = source_path::normalize(root)?;
                    uri.strip_prefix(&format!("{}/", root.trim_end_matches('/')))
                        .ok_or_else(|| error("PY001", "Source document is outside sourceRootUri"))?
                }
                // Preserve the original single-file API when no root is supplied.
                None => uri.rsplit('/').next().unwrap_or(""),
            }
        } else {
            uri.as_str()
        };
        let stem = relative
            .strip_suffix(".py")
            .ok_or_else(|| error("PY001", "Source URI must end in .py"))?;
        let segments = stem
            .split('/')
            .map(|part| {
                let name = names::identifier(part)?;
                names::module_file_stem(&name)?;
                Ok(name.to_canonical_string())
            })
            .collect::<Outcome<Vec<_>>>()?;
        if segments.first().is_some_and(|s| s == "dataclasses") {
            return Err(error("PY003", "A source module cannot shadow dataclasses"));
        }
        Ok(Self {
            python: stem.replace('/', "."),
            canonical: segments.join("/"),
        })
    }

    pub fn from_canonical(canonical: &str) -> Outcome<Self> {
        let path = ModuleName::from_canonical_string(canonical).map_err(|e| error("PY003", e))?;
        if path.is_empty() {
            return Err(error("PY003", "Module name must not be empty"));
        }
        let parts = path
            .as_path()
            .segments
            .iter()
            .map(names::module_file_stem)
            .collect::<Outcome<Vec<_>>>()?;
        let identity = Self::from_source(&format!("{}.py", parts.join("/")), None)?;
        if identity.canonical != canonical {
            return Err(error(
                "PY003",
                "Module name cannot be represented losslessly",
            ));
        }
        Ok(identity)
    }

    pub fn filename(&self) -> String {
        format!("{}.py", self.python.replace('.', "/"))
    }
}

/// Reject output collisions and module/package conflicts on every supported OS.
pub(crate) fn validate_paths<'a>(modules: impl Iterator<Item = &'a str>) -> Outcome<()> {
    let mut seen = std::collections::BTreeSet::new();
    for module in modules {
        if !seen.insert(module.to_ascii_lowercase()) {
            return Err(error(
                "PY003",
                format!("Module paths collide after normalization or case folding: {module}"),
            ));
        }
    }
    for module in &seen {
        let mut parent = module.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if seen.contains(prefix) {
                return Err(error(
                    "PY003",
                    format!("Module is also a package directory: {prefix}"),
                ));
            }
            parent = prefix;
        }
    }
    Ok(())
}

pub(crate) fn tuple_aliases<'a>(
    package: &PackageName,
    modules: impl Iterator<Item = (&'a String, &'a AccessControlled<ModuleDefinition>)>,
) -> Outcome<TupleAliases> {
    let aliases: TupleAliases =
        modules
            .flat_map(|(module, definition)| {
                definition.value.types.iter().filter_map(
                    move |(name, definition)| match &definition.value.value {
                        TypeDefinition::TypeAliasDefinition {
                            type_params,
                            type_expr: tpe @ Type::Tuple(..),
                        } if type_params.is_empty() => Some((
                            format!("{}:{module}#{name}", package.to_canonical_string()),
                            tpe.clone(),
                        )),
                        _ => None,
                    },
                )
            })
            .collect();
    for alias in aliases.values() {
        crate::values::resolve_aliases(alias, &aliases)?;
    }
    Ok(aliases)
}
