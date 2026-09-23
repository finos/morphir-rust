//! The reuse decision for one module.
//!
//! The frontend keeps no state between requests: the host hands back the
//! baseline it was given, and this module decides, per module, whether that
//! baseline still describes the module. A module is reused when its source is
//! byte-identical to the baseline's *and* none of the modules the baseline
//! recorded it depending on has a different public interface this run.
//!
//! Only interface changes propagate. A dependency whose body changed but whose
//! public interface did not cannot be observed by a dependent, so the dependent
//! keeps its baseline IR.

use std::collections::{HashMap, HashSet};

use morphir_extension_sdk::BaselineModule;

/// The two digests a module result carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digests {
    /// `sha256:` digest of the module's source text.
    pub source: String,
    /// `sha256:` digest of the module's resolved public interface.
    pub interface: String,
}

/// What to do with one module this run.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// The baseline still describes the module; reuse its IR and digests.
    Reuse(Box<BaselineModule>),
    /// The module has to be resolved and lowered again.
    Compile,
}

/// Decides whether `name` can be reused from `baseline`.
///
/// `changed_interfaces` holds the dotted names of modules whose public
/// interface differs from the baseline this run, including modules the baseline
/// knew that the request no longer contains.
pub fn decide(
    name: &[String],
    source_digest: &str,
    baseline: &HashMap<String, BaselineModule>,
    changed_interfaces: &HashSet<String>,
) -> Decision {
    let Some(entry) = baseline.get(&name.join(".")) else {
        return Decision::Compile;
    };
    if entry.source_digest != source_digest {
        return Decision::Compile;
    }
    if entry
        .depends_on
        .iter()
        .any(|dependency| changed_interfaces.contains(dependency))
    {
        return Decision::Compile;
    }
    Decision::Reuse(Box::new(entry.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, source: &str, depends_on: &[&str]) -> BaselineModule {
        BaselineModule {
            frontend_state: None,
            name: name.into(),
            uri: format!("file:///work/{name}.elm"),
            source_digest: source.into(),
            interface_digest: "sha256:interface".into(),
            depends_on: depends_on.iter().map(|d| (*d).to_string()).collect(),
            ir: serde_json::json!({}),
        }
    }

    fn baseline(entries: Vec<BaselineModule>) -> HashMap<String, BaselineModule> {
        entries
            .into_iter()
            .map(|entry| (entry.name.clone(), entry))
            .collect()
    }

    #[test]
    fn an_unknown_module_is_compiled() {
        assert_eq!(
            decide(
                &["A".into()],
                "sha256:a",
                &baseline(vec![]),
                &HashSet::new()
            ),
            Decision::Compile
        );
    }

    #[test]
    fn a_changed_source_is_compiled() {
        let baseline = baseline(vec![entry("A", "sha256:old", &[])]);
        assert_eq!(
            decide(&["A".into()], "sha256:new", &baseline, &HashSet::new()),
            Decision::Compile
        );
    }

    #[test]
    fn a_changed_dependency_interface_is_compiled() {
        let baseline = baseline(vec![entry("A", "sha256:a", &["B"])]);
        let changed = HashSet::from(["B".to_string()]);
        assert_eq!(
            decide(&["A".into()], "sha256:a", &baseline, &changed),
            Decision::Compile
        );
    }

    #[test]
    fn an_untouched_module_with_untouched_dependencies_is_reused() {
        let baseline = baseline(vec![entry("A", "sha256:a", &["B"])]);
        let changed = HashSet::from(["C".to_string()]);
        assert!(matches!(
            decide(&["A".into()], "sha256:a", &baseline, &changed),
            Decision::Reuse(reused) if reused.name == "A"
        ));
    }
}
