//! Read classic IR through the core migration, refusing collisions and annotation loss.

use crate::{Outcome, error, values};
use morphir_core::{
    ir::{classic as c, v4 as v},
    migration,
};
use std::collections::BTreeSet;

fn distinct<'a>(names: impl Iterator<Item = &'a c::Name>) -> Outcome<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        let key = migration::migrate_name(name, &Default::default())
            .map_err(|e| error("PY003", e.message))?
            .to_canonical_string();
        if !seen.insert(key) {
            return Err(error("PY003", "Duplicate name in classic IR"));
        }
    }
    Ok(())
}

pub(in crate::ir) fn decode(value: serde_json::Value) -> Outcome<v::IRFile> {
    let classic: c::Distribution =
        serde_json::from_value(value).map_err(|e| error("PY005", e.to_string()))?;
    let c::DistributionBody::Library(_, dependencies, package) = &classic.distribution else {
        return Err(values::unsupported(
            "a v3 Specs distribution has no definitions to decode",
        ));
    };
    if !dependencies.is_empty() {
        return Err(values::unsupported(
            "External dependencies are not supported",
        ));
    }
    let mut module_names = BTreeSet::new();
    for module in &package.modules {
        let key = migration::migrate_path(&module.path, &Default::default())
            .map_err(|e| error("PY003", e.message))?
            .to_canonical_string();
        if !module_names.insert(key) {
            return Err(error("PY003", "Duplicate module in classic IR"));
        }
        distinct(module.definition.value.types.iter().map(|(key, _)| key))?;
        distinct(module.definition.value.values.iter().map(|(key, _)| key))?;
        for (_, function) in &module.definition.value.values {
            let function = &function.value.value;
            distinct(function.input_types.iter().map(|arg| &arg.name))?;
        }
    }
    let migrated = migration::migrate_distribution(&classic, Default::default())
        .map_err(|e| error("PY004", format!("{}: {}", e.path, e.message)))?;
    if !migrated.report.can_publish() || !migrated.report.diagnostics().is_empty() {
        return Err(values::unsupported(
            "IR v3 migration would lose information",
        ));
    }
    let mut ir = migrated.value;
    let v::Distribution::Library(library) = &mut ir.distribution else {
        unreachable!()
    };
    let signatures = values::signatures(library)?;
    let aliases = crate::modules::tuple_aliases(&library.package_name, library.def.modules.iter())?;
    for module in &package.modules {
        for (_, function) in &module.definition.value.values {
            for arg in &function.value.value.input_types {
                let convert = |ty| {
                    migration::migrate_type(ty, &mut Default::default())
                        .map_err(|e| error("PY004", e.message))
                };
                if values::resolve_aliases(&convert(&arg.annotation)?, &aliases)?
                    != values::resolve_aliases(&convert(&arg.ty)?, &aliases)?
                {
                    return Err(values::unsupported(
                        "IR v3 parameter annotation does not match its declared type",
                    ));
                }
            }
        }
    }
    for module in library.def.modules.values_mut() {
        clear_empty_doc(&mut module.value.doc);
        for definition in module.value.types.values_mut() {
            clear_empty_doc(&mut definition.value.doc);
        }
        for definition in module.value.values.values_mut() {
            clear_empty_doc(&mut definition.value.doc);
            // Encoding checks every supplied inferred type, including intermediate comparison applications.
            super::definition(&definition.value.value, &aliases, &signatures)?;
            if let v::ValueBody::Expression(body) = &mut definition.value.value.body {
                *body = super::expressions::erase(body)?;
            }
        }
    }
    Ok(ir)
}

fn clear_empty_doc(doc: &mut Option<v::Documentation>) {
    if doc.as_ref().is_some_and(|doc| doc.text().is_empty()) {
        *doc = None;
    }
}
