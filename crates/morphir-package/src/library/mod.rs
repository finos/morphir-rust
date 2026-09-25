//! Closed Library-set integrity, using morphir-core's current IR v4 codec.

mod model;
use crate::{
    InvalidDocument,
    digest::Digest,
    local_registry::RegistryPath,
    metadata::NormalizedMetadata,
    schema::{Artifact, PackageSchemas},
    strict_json,
};
use model::{Lock, Manifest};
use morphir_core::ir::v4::{Access, Distribution, IRFile, SpellingMode, with_spelling_mode};
use morphir_core::metadata::{ContextResources, inline_document_contexts};
use std::collections::{BTreeMap, BTreeSet};

/// Untrusted Library bytes supplied by a caller. No filesystem access is performed.
#[derive(Debug, Clone)]
pub struct LibraryInput {
    manifest: String,
    files: Vec<(String, Vec<u8>)>,
}

impl LibraryInput {
    /// Preserve the manifest text and every supplied file, including duplicate paths,
    /// so verification can reject duplicates instead of silently replacing them.
    pub fn new(manifest: String, files: Vec<(String, Vec<u8>)>) -> Self {
        Self { manifest, files }
    }
}

/// A closed set whose declarations, bytes, IR payloads and graph agree.
///
/// Construction is possible only through [`Self::verify`]. This does not attest to
/// trust, acquisition, dependency resolution or public specification compatibility.
#[derive(Debug)]
pub struct VerifiedLibrarySet {
    root: String,
    manifests: BTreeMap<String, NormalizedMetadata>,
}

impl VerifiedLibrarySet {
    /// Verify a complete supplied graph against already-compiled schemas.
    ///
    /// ```
    /// use morphir_package::{library::VerifiedLibrarySet, schema::PackageSchemas};
    /// use serde_json::json;
    /// let schemas = PackageSchemas::compile(&json!({}), &json!({}))?;
    /// assert!(VerifiedLibrarySet::verify(&schemas, "{}", &[]).is_err());
    /// # Ok::<(), morphir_package::schema::SchemaError>(())
    /// ```
    pub fn verify(
        schemas: &PackageSchemas,
        lock: &str,
        libraries: &[LibraryInput],
    ) -> Result<Self, InvalidDocument> {
        strict_json::on_decode_stack(|| Self::verify_here(schemas, lock, libraries))
    }

    fn verify_here(
        schemas: &PackageSchemas,
        lock: &str,
        libraries: &[LibraryInput],
    ) -> Result<Self, InvalidDocument> {
        let normalized_lock = NormalizedMetadata::parse(lock)?;
        require(schemas.is_valid(Artifact::Lock, normalized_lock.value()))?;
        let lock: Lock =
            serde_json::from_value(normalized_lock.value().clone()).map_err(|_| InvalidDocument)?;
        require(lock.nodes.contains_key(&lock.root) && lock.nodes.len() == libraries.len())?;
        let verified = libraries
            .iter()
            .map(|input| {
                let metadata = NormalizedMetadata::parse(&input.manifest)?;
                require(schemas.is_valid(Artifact::Manifest, metadata.value()))?;
                let manifest: Manifest = serde_json::from_value(metadata.value().clone())
                    .map_err(|_| InvalidDocument)?;
                verify_payload(input, &manifest)?;
                Ok((manifest, metadata))
            })
            .collect::<Result<Vec<_>, InvalidDocument>>()?;

        let mut selected = BTreeSet::new();
        let mut manifests = BTreeMap::new();
        for (id, node) in &lock.nodes {
            let matches: Vec<_> = verified
                .iter()
                .enumerate()
                .filter(|(_, (manifest, _))| {
                    manifest.package_path == node.release.package_path
                        && manifest.version == node.release.version
                })
                .collect();
            require(matches.len() == 1)?;
            let (index, (manifest, metadata)) = matches[0];
            require(selected.insert(index))?;
            require(
                node.ir_package_name == manifest.ir.package_name
                    && node.manifest_digest == metadata.manifest_digest().to_string()
                    && node.content_digest == metadata.content_digest().to_string()
                    && node.bindings.keys().eq(manifest.dependencies.keys()),
            )?;
            for (name, requirement) in &manifest.dependencies {
                let target = lock
                    .nodes
                    .get(&node.bindings[name])
                    .ok_or(InvalidDocument)?;
                require(
                    target.ir_package_name == *name
                        && target.release.package_path == requirement.package_path,
                )?;
                let minimum = stable(&requirement.version_range.minimum_inclusive)?;
                let maximum = stable(&requirement.version_range.maximum_exclusive)?;
                let version = stable(&target.release.version)?;
                require(minimum < maximum && minimum <= version && version < maximum)?;
            }
            manifests.insert(id.clone(), metadata.clone());
        }
        Ok(Self {
            root: lock.root,
            manifests,
        })
    }

    /// The verified manifest selected as the graph root.
    pub fn root_manifest(&self) -> &NormalizedMetadata {
        &self.manifests[&self.root]
    }
    /// Verified manifests keyed by local lock node ID.
    pub fn manifests(&self) -> &BTreeMap<String, NormalizedMetadata> {
        &self.manifests
    }
}

fn require(condition: bool) -> Result<(), InvalidDocument> {
    if condition {
        Ok(())
    } else {
        Err(InvalidDocument)
    }
}

// Decimal length then lexical order compares arbitrary-size nonnegative integers exactly.
fn stable(text: &str) -> Result<Vec<(usize, &str)>, InvalidDocument> {
    let parts: Vec<_> = text.split('.').collect();
    require(parts.len() == 3)?;
    parts
        .into_iter()
        .map(|part| {
            require(
                !part.is_empty()
                    && part.bytes().all(|b| b.is_ascii_digit())
                    && (part.len() == 1 || !part.starts_with('0')),
            )?;
            Ok((part.len(), part))
        })
        .collect()
}

fn verify_payload(input: &LibraryInput, manifest: &Manifest) -> Result<(), InvalidDocument> {
    let files: BTreeMap<_, _> = input
        .files
        .iter()
        .map(|(path, bytes)| (path, bytes))
        .collect();
    require(files.len() == input.files.len() && files.keys().copied().eq(manifest.content.keys()))?;
    require(files.len() <= 129)?;
    for path in files.keys() {
        require(RegistryPath::parse(path).is_ok() && *path != "manifest.json")?;
    }
    match manifest.format_version.as_str() {
        "0.1.0-draft.1" => {
            require(manifest.context_resources.is_empty() && manifest.content.len() == 1)?;
        }
        "0.1.0-draft.2" => {
            require(
                !manifest.context_resources.is_empty()
                    && manifest.context_resources.len() + 1 == manifest.content.len(),
            )?;
            let mut total = 0usize;
            for (path, resource) in &manifest.context_resources {
                require(
                    path != &manifest.ir.payload.path
                        && resource.media_type == "application/ld+json",
                )?;
                require(manifest.content.get(path) == Some(&resource.digest))?;
                let bytes = files.get(path).ok_or(InvalidDocument)?;
                require(bytes.len() <= 1_048_576)?;
                total = total.checked_add(bytes.len()).ok_or(InvalidDocument)?;
                require(total <= 8 * 1_048_576)?;
                let context =
                    strict_json::parse(std::str::from_utf8(bytes).map_err(|_| InvalidDocument)?)
                        .map_err(|_| InvalidDocument)?;
                require(
                    context
                        .as_object()
                        .is_some_and(|value| value.len() == 1 && value.contains_key("@context")),
                )?;
            }
        }
        _ => return Err(InvalidDocument),
    }
    for (path, bytes) in &files {
        require(Digest::of_bytes(bytes).to_string() == manifest.content[*path])?;
    }
    let bytes = files
        .get(&manifest.ir.payload.path)
        .ok_or(InvalidDocument)?;
    let text = std::str::from_utf8(bytes).map_err(|_| InvalidDocument)?;
    let json = strict_json::parse(text).map_err(|_| InvalidDocument)?;
    let semantic_json = if manifest.format_version == "0.1.0-draft.2" {
        let mut resources = ContextResources::new(".");
        for (path, bytes) in &files {
            if *path != &manifest.ir.payload.path {
                resources.insert_local((*path).clone(), (*bytes).clone());
            }
        }
        inline_document_contexts(&json, &resources, Some(&manifest.ir.payload.path))
            .map_err(|_| InvalidDocument)?
    } else {
        json.clone()
    };
    let (file, warnings) = with_spelling_mode(SpellingMode::Current, || {
        serde_json::from_value::<IRFile>(semantic_json)
    });
    let file = file.map_err(|_| InvalidDocument)?;
    let release = file
        .format_version
        .normalize()
        .map_err(|_| InvalidDocument)?
        .release;
    require(release.major() == 4)?;
    if manifest.format_version == "0.1.0-draft.2" {
        require(release.minor() >= 1)?;
    }
    require(warnings.is_empty())?;
    let Distribution::Library(library) = file.distribution else {
        return Err(InvalidDocument);
    };
    require(library.package_name.to_canonical_string() == manifest.ir.package_name)?;
    let dependencies: BTreeSet<_> = library.dependencies.keys().collect();
    require(dependencies.into_iter().eq(manifest.dependencies.keys()))?;
    for target in manifest.exports.values() {
        let module = library.def.modules.get(target).ok_or(InvalidDocument)?;
        require(module.access == Access::Public)?;
        // The draft export contract names the canonical Public wrapper.
        require(
            json["distribution"]["Library"]["def"]["modules"][target]
                .get("Public")
                .is_some(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::stable;

    #[test]
    fn stable_components_are_unbounded_integers() {
        assert!(
            stable("999999999999999999999999999999.0.0").unwrap()
                < stable("1000000000000000000000000000000.0.0").unwrap()
        );
        assert!(stable("1.9007199254740992.0").unwrap() < stable("1.9007199254740993.0").unwrap());
        for invalid in ["01.0.0", "1.0", "1.0.0.0", "1.0.-1", "1.0.0-beta"] {
            assert!(stable(invalid).is_err());
        }
    }
}
