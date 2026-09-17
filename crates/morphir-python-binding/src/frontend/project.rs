//! Compile a closed set of source modules in declaration and value passes.

use super::{at, imports, lower};
use crate::{
    Outcome, error,
    modules::{self, ModuleIdentity},
};
use morphir_core::ir::v4::*;
use morphir_extension_sdk::CompileRequest;
use ruff_text_size::Ranged;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn compile(request: &CompileRequest) -> Outcome<(serde_json::Value, Vec<String>)> {
    if request.documents.is_empty() {
        return Err(error("PY001", "Expected at least one Python source module"));
    }
    let package =
        PackageName::from_canonical_string(&request.package.name).map_err(|e| error("PY001", e))?;
    if package.is_empty() {
        return Err(error("PY001", "Package name must not be empty"));
    }
    let paths = request
        .source_paths()
        .map_err(|e| error("PY001", e.to_string()))?;
    let parsed = request
        .documents
        .iter()
        .zip(&paths)
        .map(|(document, path)| {
            if document.language_id != "python" {
                return Err(error("PY001", "Source document language must be python"));
            }
            let identity = ModuleIdentity::from_relative_path(path.as_str())?;
            let syntax = ruff_python_parser::parse_module(&document.text).map_err(|e| {
                at(
                    error("PY002", e.to_string()),
                    &document.uri,
                    &document.text,
                    e.range(),
                )
            })?;
            Ok((document, identity, syntax))
        })
        .collect::<Outcome<Vec<_>>>()?;
    modules::validate_paths(parsed.iter().map(|(_, id, _)| id.canonical.as_str()))?;
    let names: BTreeSet<_> = parsed
        .iter()
        .map(|(_, id, _)| id.canonical.clone())
        .collect();
    let exposed = request
        .package
        .exposed_modules
        .iter()
        .flatten()
        .map(|name| ModuleName::parse(&name.replace('.', "/")).to_canonical_string())
        .collect::<BTreeSet<_>>();
    if !exposed.is_subset(&names) {
        return Err(error(
            "PY001",
            "exposedModules names an unknown source module",
        ));
    }
    let exports = parsed
        .iter()
        .map(|(document, id, syntax)| {
            imports::exports(syntax.suite(), &package, &id.canonical)
                .map(|types| (id.python.clone(), types))
                .map_err(|e| at(e, &document.uri, &document.text, syntax.syntax().range()))
        })
        .collect::<Outcome<_>>()?;
    let scopes = parsed
        .iter()
        .map(|(document, id, syntax)| {
            imports::scope(syntax.suite(), id, &exports)
                .map_err(|e| at(e, &document.uri, &document.text, syntax.syntax().range()))
        })
        .collect::<Outcome<Vec<_>>>()?;
    let lower_modules = |aliases| -> Outcome<BTreeMap<_, _>> {
        parsed
            .iter()
            .zip(&scopes)
            .map(|((document, id, syntax), scope)| {
                lower(syntax.suite(), scope, aliases)
                    .map(|module| {
                        (
                            id.canonical.clone(),
                            AccessControlled {
                                access: if request.package.exposed_modules.is_none()
                                    || exposed.contains(&id.canonical)
                                {
                                    Access::Public
                                } else {
                                    Access::Private
                                },
                                value: module,
                            },
                        )
                    })
                    .map_err(|e| at(e, &document.uri, &document.text, syntax.syntax().range()))
            })
            .collect()
    };
    let definitions = lower_modules(None)?;
    let aliases = modules::tuple_aliases(&package, definitions.iter())?;
    let definitions = lower_modules(Some(&aliases))?;
    if request.options.types_only && definitions.values().any(|m| !m.value.values.is_empty()) {
        return Err(error(
            "PY004",
            "typesOnly is not supported for Python functions; compile with typesOnly false",
        ));
    }
    let ir = IRFile {
        format_version: FormatVersion::Integer(4),
        distribution: Distribution::Library(LibraryContent {
            package_name: package,
            dependencies: Default::default(),
            def: PackageDefinition {
                modules: definitions.into_iter().collect(),
            },
        }),
    };
    Ok((
        with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(ir))
            .map_err(|e| error("PY005", e.to_string()))?,
        names.into_iter().collect(),
    ))
}
