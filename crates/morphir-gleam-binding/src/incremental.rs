//! Stateless baseline validation and interface-sensitive reuse.
use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use morphir_core::ir::v4::{
    Access, AccessControlled, ModuleDefinition, ModuleSpecification, PackageDefinition,
    PackageSpecification,
};
use morphir_extension_sdk::{BaselineModule, CompileRequest, Diagnostic, DiagnosticSeverity};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) struct Baseline {
    pub(crate) modules: HashMap<String, BaselineModule>,
    pub(crate) definitions: HashMap<String, AccessControlled<ModuleDefinition>>,
    pub(crate) dropped: HashSet<String>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decision {
    Reuse(BaselineModule),
    Compile,
}

/// Hash the exact source bytes, including documentation and whitespace.
pub(crate) fn source_digest(source: &str) -> String {
    // sha2 0.11 digests are `hybrid_array::Array`s, which do not implement `LowerHex`.
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in Sha256::digest(source.as_bytes()) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Hash only the module's public shape and access, omitting implementation and docs.
///
/// A private module still exposes public members to other modules in its package.
/// Temporarily treating it as public derives that interface without dropping it.
pub(crate) fn interface_digest(module: &AccessControlled<ModuleDefinition>) -> String {
    let package = PackageDefinition {
        modules: IndexMap::from([(
            "module".into(),
            AccessControlled {
                access: Access::Public,
                value: module.value.clone(),
            },
        )]),
    };
    let mut specification = package.to_specification();
    remove_docs(&mut specification);
    digest_json(&json!({"access": module.access, "specification": specification}))
}

/// Identify all inputs to resolution, independently of source and output paths.
pub(crate) fn context_digest(
    request: &CompileRequest,
    dependencies: &IndexMap<String, PackageSpecification>,
) -> String {
    let dependencies: IndexMap<_, _> = dependencies
        .iter()
        .map(|(name, specification)| {
            let mut specification = specification.clone();
            remove_docs(&mut specification);
            (name.clone(), specification)
        })
        .collect();
    let options: std::collections::BTreeMap<_, _> = request
        .options
        .extra
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "outputDir" | "emitParseStage" | "emitParseStageFatal"
            )
        })
        .collect();
    let exposed = request.package.exposed_modules.as_ref().map(|names| {
        let mut names = names.clone();
        names.sort_unstable();
        names.dedup();
        names
    });
    digest_json(&json!({
        "frontend": concat!("morphir-gleam-binding/", env!("CARGO_PKG_VERSION"), "/incremental-v1"),
        "package": request.package.name,
        "exposed": exposed,
        "language": request.language_id,
        "irVersion": request.options.ir_version,
        "typesOnly": request.options.types_only,
        "options": options,
        "dependencies": dependencies,
    }))
}

/// Validate every entry before either reusing IR or restoring a failed module's interface.
pub(crate) fn read_baseline(
    request: &CompileRequest,
    context: &str,
    decode: impl Fn(&Value) -> Result<AccessControlled<ModuleDefinition>, String>,
) -> Baseline {
    let mut baseline = Baseline {
        modules: HashMap::new(),
        definitions: HashMap::new(),
        dropped: HashSet::new(),
        diagnostics: Vec::new(),
    };
    let Some(supplied) = &request.baseline else {
        return baseline;
    };
    match supplied.context_digest.as_deref() {
        Some(recorded) if recorded == context => {}
        context => {
            baseline.diagnostics.push(warning(if context.is_some() {
                "baseline ignored: it was built under a different compile context".into()
            } else {
                "baseline ignored: it carries no contextDigest".into()
            }));
            return baseline;
        }
    }
    let mut names = HashSet::new();
    for entry in &supplied.modules {
        if !names.insert(&entry.name) && baseline.dropped.insert(entry.name.clone()) {
            baseline.diagnostics.push(warning(format!(
                "baseline for module {} ignored: duplicate module name",
                entry.name
            )));
        }
    }
    for entry in &supplied.modules {
        if baseline.dropped.contains(&entry.name) {
            continue;
        }
        match decode(&entry.ir).and_then(|definition| {
            if interface_digest(&definition) == entry.interface_digest {
                Ok(definition)
            } else {
                Err("interface digest does not match the stored module".into())
            }
        }) {
            Ok(definition) => {
                baseline.definitions.insert(entry.name.clone(), definition);
                baseline.modules.insert(entry.name.clone(), entry.clone());
            }
            Err(reason) => {
                baseline.dropped.insert(entry.name.clone());
                baseline.diagnostics.push(warning(format!(
                    "baseline for module {} ignored: {reason}",
                    entry.name
                )));
            }
        }
    }
    baseline
}

/// Reuse requires identical source and unchanged interfaces for every recorded import.
pub(crate) fn decide(
    name: &str,
    source_digest: &str,
    baseline: &HashMap<String, BaselineModule>,
    changed_interfaces: &HashSet<String>,
) -> Decision {
    match baseline.get(name) {
        Some(entry)
            if entry.source_digest == source_digest
                && !entry
                    .depends_on
                    .iter()
                    .any(|name| changed_interfaces.contains(name)) =>
        {
            Decision::Reuse(entry.clone())
        }
        _ => Decision::Compile,
    }
}

fn remove_docs(package: &mut PackageSpecification) {
    for ModuleSpecification {
        doc, types, values, ..
    } in package.modules.values_mut()
    {
        *doc = None;
        for value in types.values_mut() {
            value.doc = None;
        }
        for value in values.values_mut() {
            value.doc = None;
        }
    }
}

fn warning(message: String) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Warning,
        code: Some("GLEAM_BASELINE".into()),
        message,
        location: None,
        related: Vec::new(),
    }
}

fn digest_json(value: &Value) -> String {
    // Sorting explicitly also works when another crate enables serde_json's preserve_order.
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(members) => {
                let sorted: std::collections::BTreeMap<_, _> = members.iter().collect();
                Value::Object(
                    sorted
                        .into_iter()
                        .map(|(key, value)| (key.clone(), canonical(value)))
                        .collect(),
                )
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            other => other.clone(),
        }
    }
    source_digest(&canonical(value).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_core::ir::v4::{Access, Documented, Type, TypeDefinition};
    use morphir_extension_sdk::CompileBaseline;

    fn empty_module() -> AccessControlled<ModuleDefinition> {
        AccessControlled {
            access: Access::Public,
            value: ModuleDefinition {
                types: IndexMap::new(),
                values: IndexMap::new(),
                doc: None,
            },
        }
    }

    fn stored_module(name: &str, dependencies: &[&str]) -> BaselineModule {
        let module = empty_module();
        BaselineModule {
            name: name.into(),
            uri: format!("file:///src/{name}.gleam"),
            source_digest: source_digest("pub type X = Int"),
            interface_digest: interface_digest(&module),
            depends_on: dependencies.iter().map(|name| (*name).into()).collect(),
            ir: serde_json::to_value(module).unwrap(),
        }
    }

    fn decode(value: &serde_json::Value) -> Result<AccessControlled<ModuleDefinition>, String> {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }

    #[test]
    fn source_digest_matches_sha256_known_vector() {
        assert_eq!(
            source_digest("abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn changed_dependency_or_source_requires_compilation() {
        let entry = stored_module("domain/a", &["domain/b"]);
        let baseline = HashMap::from([(entry.name.clone(), entry.clone())]);
        assert_eq!(
            decide(
                &entry.name,
                &entry.source_digest,
                &baseline,
                &HashSet::new()
            ),
            Decision::Reuse(entry.clone())
        );
        assert_eq!(
            decide(&entry.name, "changed", &baseline, &HashSet::new()),
            Decision::Compile
        );
        assert_eq!(
            decide(
                &entry.name,
                &entry.source_digest,
                &baseline,
                &HashSet::from(["domain/b".into()])
            ),
            Decision::Compile
        );
    }

    #[test]
    fn malformed_or_tampered_entries_are_dropped_with_diagnostics() {
        let mut malformed = stored_module("broken", &[]);
        malformed.ir = serde_json::json!({"bad": true});
        let mut tampered = stored_module("tampered", &[]);
        tampered.interface_digest = "sha256:wrong".into();
        let request = CompileRequest {
            baseline: Some(CompileBaseline {
                context_digest: Some("same".into()),
                modules: vec![malformed, tampered, stored_module("valid", &[])],
            }),
            ..Default::default()
        };
        let baseline = read_baseline(&request, "same", decode);
        assert_eq!(baseline.modules.len(), 1);
        assert!(baseline.definitions.contains_key("valid"));
        assert_eq!(
            baseline.dropped,
            HashSet::from(["broken".into(), "tampered".into()])
        );
        assert_eq!(baseline.diagnostics.len(), 2);
    }

    #[test]
    fn absent_or_changed_context_discards_the_whole_baseline() {
        for context in [None, Some("other".into())] {
            let request = CompileRequest {
                baseline: Some(CompileBaseline {
                    context_digest: context,
                    modules: vec![stored_module("a", &[])],
                }),
                ..Default::default()
            };
            let baseline = read_baseline(&request, "same", decode);
            assert!(baseline.modules.is_empty());
            assert_eq!(baseline.diagnostics.len(), 1);
        }
    }

    #[test]
    fn duplicate_module_names_make_both_baseline_entries_unusable() {
        let entry = stored_module("a", &[]);
        let request = CompileRequest {
            baseline: Some(CompileBaseline {
                context_digest: Some("same".into()),
                modules: vec![entry.clone(), entry],
            }),
            ..Default::default()
        };
        let baseline = read_baseline(&request, "same", decode);
        assert!(baseline.modules.is_empty());
        assert!(baseline.definitions.is_empty());
        assert!(baseline.dropped.contains("a"));
        assert_eq!(baseline.diagnostics.len(), 1);
    }

    #[test]
    fn module_docs_and_private_members_are_not_interface_changes() {
        let original = empty_module();
        let mut edited = original.clone();
        edited.value.doc = Some("Module docs".into());
        edited.value.types.insert(
            "hidden".into(),
            AccessControlled {
                access: Access::Private,
                value: Documented::new(
                    None,
                    TypeDefinition::TypeAliasDefinition {
                        type_params: vec![],
                        type_expr: Type::Unit(Default::default()),
                    },
                ),
            },
        );
        assert_eq!(interface_digest(&original), interface_digest(&edited));
        edited.value.types.get_mut("hidden").unwrap().access = Access::Public;
        assert_ne!(interface_digest(&original), interface_digest(&edited));
        let before_docs = interface_digest(&edited);
        edited.value.types.get_mut("hidden").unwrap().value.doc = Some("Type docs".into());
        assert_eq!(before_docs, interface_digest(&edited));
    }

    #[test]
    fn private_module_public_types_still_affect_in_package_dependents() {
        let mut module = empty_module();
        module.access = Access::Private;
        let original = interface_digest(&module);
        module.value.types.insert(
            "visible".into(),
            AccessControlled {
                access: Access::Public,
                value: Documented::new(
                    None,
                    TypeDefinition::TypeAliasDefinition {
                        type_params: vec![],
                        type_expr: Type::Unit(Default::default()),
                    },
                ),
            },
        );
        assert_ne!(original, interface_digest(&module));
    }

    #[test]
    fn output_location_does_not_change_compilation_context() {
        let mut request = CompileRequest::default();
        let before = context_digest(&request, &IndexMap::new());
        request
            .options
            .extra
            .insert("outputDir".into(), "/new/output".into());
        request
            .options
            .extra
            .insert("emitParseStage".into(), false.into());
        assert_eq!(before, context_digest(&request, &IndexMap::new()));
        request.options.types_only = true;
        assert_ne!(before, context_digest(&request, &IndexMap::new()));
    }

    #[test]
    fn dependency_interface_changes_change_context_but_docs_and_order_do_not() {
        let specification = PackageDefinition {
            modules: IndexMap::from([("types".into(), empty_module())]),
        }
        .to_specification();
        let mut dependencies = IndexMap::from([
            ("acme".into(), specification.clone()),
            ("other".into(), specification),
        ]);
        let request = CompileRequest::default();
        let original = context_digest(&request, &dependencies);
        dependencies.reverse();
        dependencies
            .get_mut("acme")
            .unwrap()
            .modules
            .get_mut("types")
            .unwrap()
            .doc = Some("Docs".into());
        assert_eq!(original, context_digest(&request, &dependencies));
        dependencies.get_mut("acme").unwrap().modules.clear();
        assert_ne!(original, context_digest(&request, &dependencies));
    }

    #[test]
    fn type_named_doc_remains_part_of_the_interface() {
        use morphir_core::ir::v4::TypeSpecification;
        let specification = |type_expr| PackageSpecification {
            modules: IndexMap::from([(
                "types".into(),
                ModuleSpecification {
                    annotations: Vec::new(),
                    doc: None,
                    values: IndexMap::new(),
                    types: IndexMap::from([(
                        "doc".into(),
                        Documented::new(
                            None,
                            TypeSpecification::TypeAliasSpecification {
                                annotations: Vec::new(),
                                type_params: Vec::new(),
                                type_expr,
                            },
                        ),
                    )]),
                },
            )]),
        };
        let unit = Type::Unit(Default::default());
        let tuple = Type::Tuple(Default::default(), vec![unit.clone()]);
        let first = IndexMap::from([("acme".into(), specification(unit))]);
        let second = IndexMap::from([("acme".into(), specification(tuple))]);
        assert_ne!(
            context_digest(&CompileRequest::default(), &first),
            context_digest(&CompileRequest::default(), &second)
        );
    }
}
