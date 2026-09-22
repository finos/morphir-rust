use serde::Serialize;

/// A primitive failed validation at a local-registry boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid local-registry value")]
pub struct ValueError;
macro_rules! text_value {
    ($name:ident, $doc:literal, $valid:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);
        impl $name {
            /// Validate the wire spelling before entering the domain.
            pub fn parse(text: &str) -> Result<Self, ValueError> {
                ($valid)(text)
                    .then(|| Self(text.to_owned()))
                    .ok_or(ValueError)
            }
            /// The validated wire spelling.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}
fn lower_segment(text: &str) -> bool {
    !text.is_empty()
        && text.split('-').all(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}
text_value!(LocalId, "A local registry or evidence alias.", |s: &str| s
    .as_bytes()
    .first()
    .is_some_and(u8::is_ascii_lowercase)
    && lower_segment(s));
text_value!(
    RegistryPath,
    "A validated portable path relative to a registry.",
    |s: &str| registry_path_fault(s).is_none()
);
text_value!(
    Namespace,
    "A package namespace grant, matched by path components.",
    |s: &str| {
        let mut parts = s.split('/');
        let domain = parts.next().unwrap_or_default();
        domain.contains('.') && domain.split('.').all(lower_segment) && parts.all(lower_segment)
    }
);
text_value!(
    PublisherKey,
    "An Ed25519 public key's canonical lowercase hex spelling.",
    |s: &str| s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
);
/// Reuse the resolution contract's validated SHA-256 digest.
pub type Digest = crate::resolution::ResolutionDigest;
/// Why a registry path is outside the portable profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryPathFault {
    /// A portable resource limit was exceeded.
    Resource {
        resource: super::Resource,
        maximum: usize,
    },
    /// The path spelling is unsafe.
    Path(PathRule),
}
/// The violated portable-path constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PathRule {
    /// Invalid relative path grammar.
    Grammar,
    /// Windows device basename.
    ReservedName,
}
/// Diagnose a portable path, preserving resource-priority order.
pub fn registry_path_fault(text: &str) -> Option<RegistryPathFault> {
    use super::Resource;
    if text.len() > 240 {
        return Some(RegistryPathFault::Resource {
            resource: Resource::PathBytes,
            maximum: 240,
        });
    }
    let parts: Vec<_> = text.split('/').collect();
    if parts.len() > 32 {
        return Some(RegistryPathFault::Resource {
            resource: Resource::PathComponents,
            maximum: 32,
        });
    }
    if parts.iter().any(|s| s.len() > 128) {
        return Some(RegistryPathFault::Resource {
            resource: Resource::ComponentBytes,
            maximum: 128,
        });
    }
    if !parts.iter().all(|s| {
        !s.is_empty()
            && s.split(['.', '-']).all(|s| {
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            })
    }) {
        return Some(RegistryPathFault::Path(PathRule::Grammar));
    }
    if parts.iter().any(|s| {
        let base = s.split('.').next().unwrap_or_default();
        matches!(base, "con" | "prn" | "aux" | "nul")
            || (base.len() == 4
                && (base.starts_with("com") || base.starts_with("lpt"))
                && base.as_bytes()[3].is_ascii_digit())
    }) {
        return Some(RegistryPathFault::Path(PathRule::ReservedName));
    }
    None
}
/// A valid object location supplied by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectSubject {
    /// Local registry alias.
    pub registry: LocalId,
    /// Portable relative path.
    pub path: RegistryPath,
}
/// Logical input being interpreted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Subject {
    /// A lock document.
    Lock,
    /// Trusted local policy.
    Policy,
    /// An operation request.
    Request,
    /// A registry's metadata.
    Repository { registry: LocalId },
    /// A registry object.
    Object {
        registry: LocalId,
        path: RegistryPath,
    },
}
impl From<&ObjectSubject> for Subject {
    fn from(s: &ObjectSubject) -> Self {
        Self::Object {
            registry: s.registry.clone(),
            path: s.path.clone(),
        }
    }
}
/// A diagnostic may describe an invalid path that cannot enter `RegistryPath`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SubjectWire {
    /// Lock input.
    Lock,
    /// Local policy input.
    Policy,
    /// Request input.
    Request,
    /// Registry input.
    Repository { registry: String },
    /// Object input, potentially containing an unsafe path.
    Object { registry: String, path: String },
}
impl From<&Subject> for SubjectWire {
    fn from(s: &Subject) -> Self {
        match s {
            Subject::Lock => Self::Lock,
            Subject::Policy => Self::Policy,
            Subject::Request => Self::Request,
            Subject::Repository { registry } => Self::Repository {
                registry: registry.as_str().into(),
            },
            Subject::Object { registry, path } => Self::Object {
                registry: registry.as_str().into(),
                path: path.as_str().into(),
            },
        }
    }
}
