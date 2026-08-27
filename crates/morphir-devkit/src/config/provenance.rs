use super::sources::ConfigSourceKind;
use morphir_common::config::{ProvenanceMap, deep_merge_with_provenance};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigOrigin {
    pub(crate) kind: ConfigSourceKind,
    pub(crate) path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ConfigProvenance {
    origins: ProvenanceMap<ConfigOrigin>,
}

impl ConfigProvenance {
    pub(crate) fn origin(&self, path: &[String]) -> Option<&ConfigOrigin> {
        self.origins.get(path)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProvenanceState {
    value: Value,
    provenance: ConfigProvenance,
}

impl ProvenanceState {
    pub(crate) fn merge(&mut self, overlay: &Value, origin: ConfigOrigin) {
        let (value, origins) =
            deep_merge_with_provenance(&self.value, &self.provenance.origins, overlay, &origin);
        self.value = value;
        self.provenance.origins = origins;
    }

    #[cfg(test)]
    fn origin(&self, path: &[String]) -> Option<&ConfigOrigin> {
        self.provenance.origin(path)
    }

    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn into_parts(self) -> (Value, ConfigProvenance) {
        (self.value, self.provenance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSourceKind;
    use serde_json::json;
    use std::path::PathBuf;

    fn origin(kind: ConfigSourceKind, path: &str) -> ConfigOrigin {
        ConfigOrigin {
            kind,
            path: Some(PathBuf::from(path)),
        }
    }

    #[test]
    fn tracks_the_winning_file_for_each_leaf() {
        let project = origin(ConfigSourceKind::Project, "/work/morphir.toml");
        let user = origin(ConfigSourceKind::UserOverride, "/work/morphir.user.toml");
        let mut state = ProvenanceState::default();

        state.merge(
            &json!({"registry": {"endpoint": "https://example", "token": {"env": "OLD"}}}),
            project.clone(),
        );
        state.merge(
            &json!({"registry": {"token": {"command": ["gh", "auth", "token"]}}}),
            user.clone(),
        );

        assert_eq!(
            state.origin(&["registry".into(), "endpoint".into()]),
            Some(&project)
        );
        assert_eq!(
            state.origin(&["registry".into(), "token".into()]),
            Some(&user)
        );
    }

    #[test]
    fn records_one_origin_for_a_replaced_array() {
        let project = origin(ConfigSourceKind::Project, "/work/morphir.toml");
        let user = origin(ConfigSourceKind::UserOverride, "/work/morphir.user.toml");
        let mut state = ProvenanceState::default();

        state.merge(&json!({"registries": ["https://old"]}), project);
        state.merge(&json!({"registries": ["https://new"]}), user.clone());

        assert_eq!(state.origin(&["registries".into()]), Some(&user));
        assert_eq!(state.origin(&["registries".into(), "0".into()]), None);
    }

    #[test]
    fn replacement_removes_stale_descendant_origins() {
        let project = origin(ConfigSourceKind::Project, "/work/morphir.toml");
        let user = origin(ConfigSourceKind::UserOverride, "/work/morphir.user.toml");
        let mut state = ProvenanceState::default();

        state.merge(&json!({"token": {"env": "OLD"}}), project);
        state.merge(&json!({"token": "literal"}), user.clone());

        assert_eq!(state.origin(&["token".into()]), Some(&user));
        assert_eq!(state.origin(&["token".into(), "env".into()]), None);
    }
}
