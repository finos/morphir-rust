use super::values::*;
use serde::Serialize;

/// One selected dependency binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub(crate) ir_package_name: IrPackageName,
    pub(crate) target: ReleaseId,
}
impl Binding {
    /// The dependency IR name.
    pub fn ir_package_name(&self) -> &IrPackageName {
        &self.ir_package_name
    }
    /// The selected release identity.
    pub fn target(&self) -> &ReleaseId {
        &self.target
    }
}

/// One selected node and the immutable metadata used to select it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedNode {
    pub(crate) release: ReleaseId,
    pub(crate) ir_package_name: IrPackageName,
    pub(crate) manifest_digest: ResolutionDigest,
    pub(crate) content_digest: ResolutionDigest,
    pub(crate) bindings: Vec<Binding>,
}
impl LockedNode {
    /// The selected release identity.
    pub fn release(&self) -> &ReleaseId {
        &self.release
    }
    /// The selected release's IR name.
    pub fn ir_package_name(&self) -> &IrPackageName {
        &self.ir_package_name
    }
    /// Bindings sorted by IR package name.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The immutable manifest metadata digest.
    pub fn manifest_digest(&self) -> &ResolutionDigest {
        &self.manifest_digest
    }

    /// The immutable package-content digest.
    pub fn content_digest(&self) -> &ResolutionDigest {
        &self.content_digest
    }
}

/// A normalized, reachable, acyclic flat Library graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LockedGraph {
    pub(crate) root: ReleaseId,
    pub(crate) nodes: Vec<LockedNode>,
}
impl LockedGraph {
    /// The fixed root identity.
    pub fn root(&self) -> &ReleaseId {
        &self.root
    }
    /// Root first, followed by nodes in canonical release order.
    pub fn nodes(&self) -> &[LockedNode] {
        &self.nodes
    }
}

/// Immutable metadata for one exact release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseRecord {
    pub(crate) release: ReleaseId,
    pub(crate) ir_package_name: IrPackageName,
    pub(crate) manifest_digest: ResolutionDigest,
    pub(crate) content_digest: ResolutionDigest,
    pub(crate) dependencies: Vec<Requirement>,
}
impl ReleaseRecord {
    /// The exact release identity.
    pub fn release(&self) -> &ReleaseId {
        &self.release
    }
    /// The unversioned IR package name.
    pub fn ir_package_name(&self) -> &IrPackageName {
        &self.ir_package_name
    }
    /// Requirements declared by this release.
    pub fn dependencies(&self) -> &[Requirement] {
        &self.dependencies
    }

    /// The immutable manifest metadata digest.
    pub fn manifest_digest(&self) -> &ResolutionDigest {
        &self.manifest_digest
    }

    /// The immutable package-content digest.
    pub fn content_digest(&self) -> &ResolutionDigest {
        &self.content_digest
    }
}

/// One dependency requirement declared by a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub(crate) ir_package_name: IrPackageName,
    pub(crate) package_path: PackagePath,
    pub(crate) version_range: VersionRange,
}
impl Requirement {
    /// The required IR package name.
    pub fn ir_package_name(&self) -> &IrPackageName {
        &self.ir_package_name
    }
    /// The authority-bearing required path.
    pub fn package_path(&self) -> &PackagePath {
        &self.package_path
    }
    /// The eligible stable-version interval.
    pub fn version_range(&self) -> &VersionRange {
        &self.version_range
    }
}

/// An inclusive-minimum, exclusive-maximum stable-version interval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRange {
    pub(crate) minimum_inclusive: StableVersion,
    pub(crate) maximum_exclusive: StableVersion,
}
impl VersionRange {
    /// Inclusive lower bound.
    pub fn minimum_inclusive(&self) -> &StableVersion {
        &self.minimum_inclusive
    }
    /// Exclusive upper bound.
    pub fn maximum_exclusive(&self) -> &StableVersion {
        &self.maximum_exclusive
    }
    pub(crate) fn contains(&self, version: &StableVersion) -> bool {
        self.minimum_inclusive <= *version && *version < self.maximum_exclusive
    }
}

/// The complete candidate list declared for one package path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Catalog {
    #[serde(rename = "packagePath")]
    pub(crate) package_path: PackagePath,
    pub(crate) releases: Vec<ReleaseRecord>,
}
impl Catalog {
    /// The catalog's authority-bearing path.
    pub fn package_path(&self) -> &PackagePath {
        &self.package_path
    }
    /// The immutable candidate records.
    pub fn releases(&self) -> &[ReleaseRecord] {
        &self.releases
    }
}

/// An eligible or exact update request for one old locked path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UpdateTarget {
    /// Select the freshest feasible version.
    Eligible {
        #[serde(rename = "packagePath")]
        package_path: PackagePath,
    },
    /// Require one exact version.
    Exact {
        #[serde(rename = "packagePath")]
        package_path: PackagePath,
        version: StableVersion,
    },
}
impl UpdateTarget {
    /// The old locked path requested for update.
    pub fn package_path(&self) -> &PackagePath {
        match self {
            Self::Eligible { package_path } | Self::Exact { package_path, .. } => package_path,
        }
    }

    /// The exact version, or `None` for an eligible target.
    pub fn exact_version(&self) -> Option<&StableVersion> {
        match self {
            Self::Eligible { .. } => None,
            Self::Exact { version, .. } => Some(version),
        }
    }
}
