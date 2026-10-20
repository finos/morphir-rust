//! Read-only acquisition through the actual selected root, without fallback.
mod native;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod special_file_tests;
use super::{Attempt, FileIdentity, ObservedContentFile, ReadClass, SourceReadScope};
use cap_std::fs::Dir;
use std::{
    io,
    path::{Component, Path, PathBuf},
};

/// Failure to issue or acquire a confined, identity-pinned read target.
#[derive(Debug, thiserror::Error)]
pub enum RootReadError {
    /// Paths must contain only nonempty normal relative components.
    #[error("invalid root-relative path")]
    InvalidRelativePath,
    /// The running platform cannot enforce the required native resolution.
    #[error("native root confinement is unavailable")]
    UnsupportedPlatform,
    /// The plan belongs to another source owner.
    #[error("foreign root-read plan")]
    ForeignScope,
    /// Actual descriptor identity differs from the issued target/grant.
    #[error("root-read identity changed")]
    IdentityMismatch,
    /// Native confinement refused a link, reparse point or resolution race.
    #[error("native root confinement refused acquisition")]
    ConfinementRefused,
    /// Only regular disk files can be returned for content reading.
    #[error("root-read target is not a regular file")]
    NonRegular,
    /// Real metadata, allocation or acquisition failure.
    #[error("root-read I/O: {0}")]
    Io(#[from] io::Error),
}

/// Validated, nonempty normal relative file path. It grants no access.
#[derive(Debug)]
pub struct RootRelativeFile(PathBuf);
impl RootRelativeFile {
    /// Preserve exact platform path spelling while rejecting ambiguous routes.
    pub fn parse(path: &Path) -> Result<Self, RootReadError> {
        let bytes = path.as_os_str().as_encoded_bytes();
        let separator = |byte: &u8| *byte == b'/' || cfg!(windows) && *byte == b'\\';
        if bytes.is_empty()
            || bytes.contains(&0)
            || cfg!(windows) && bytes.contains(&b':')
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || bytes
                .split(separator)
                .any(|part| part.is_empty() || part == b"." || part == b"..")
        {
            return Err(RootReadError::InvalidRelativePath);
        }
        let mut owned = PathBuf::new();
        owned
            .try_reserve_exact(bytes.len())
            .map_err(io::Error::other)?;
        owned.push(path);
        Ok(Self(owned))
    }
}
/// An output grant can be the actual selected root or a proper descendant.
#[derive(Debug)]
pub enum RootRelativeDirectory {
    /// The output is the selected workspace root itself.
    SelectedRoot,
    /// A validated, nonempty normal relative directory route.
    Descendant(RootRelativeFile),
}
impl RootRelativeDirectory {
    /// Validate a proper descendant; use SelectedRoot for the root itself.
    pub fn descendant(path: &Path) -> Result<Self, RootReadError> {
        RootRelativeFile::parse(path).map(Self::Descendant)
    }
}
/// Opaque expected incarnation retained from a genuine output grant.
/// Metadata use does not issue a retrospective file-read witness.
pub struct ReadOnlyRootPlan<'scope, 'hooks> {
    scope: &'scope SourceReadScope,
    relative: PathBuf,
    expected: FileIdentity,
    #[cfg(test)]
    pub(super) before_data_acquire: Option<&'hooks dyn Fn()>,
    #[cfg(test)]
    pub(super) after_data_acquire: Option<&'hooks dyn Fn()>,
    _hooks: std::marker::PhantomData<&'hooks ()>,
}
/// A real confined content descriptor, using the shared observed Read loop.
pub type RootAcquiredRead<'scope> = ObservedContentFile<'scope>;

impl SourceReadScope {
    /// Pin an expected regular-file incarnation from an already-held output
    /// grant. Neither path operands nor this plan create host authority.
    ///
    /// ```no_run
    /// # use morphir_common::config::observation::*;
    /// # use cap_std::fs::Dir;
    /// # use std::path::Path;
    /// # fn example(scope: &SourceReadScope, output: &Dir) -> Result<(), RootReadError> {
    /// let directory = RootRelativeDirectory::descendant(Path::new(".morphir"))?;
    /// let file = RootRelativeFile::parse(Path::new("compile.dest/model.json"))?;
    /// let plan = scope.prepare_root_read(output, &directory, &file)?;
    /// let acquired = scope.open_read_only_root(&plan)?;
    /// assert!(acquired.metadata()?.is_file());
    /// # Ok(()) }
    /// ```
    pub fn prepare_root_read<'scope, 'hooks>(
        &'scope self,
        output: &Dir,
        output_route: &RootRelativeDirectory,
        file: &RootRelativeFile,
    ) -> Result<ReadOnlyRootPlan<'scope, 'hooks>, RootReadError> {
        if !native::supported() {
            return Err(RootReadError::UnsupportedPlatform);
        }
        native::ensure_support(&self.directory)?;
        let relative = match output_route {
            RootRelativeDirectory::SelectedRoot => {
                if FileIdentity::from_file(output.try_clone()?.into_std_file())? != self.identity {
                    return Err(RootReadError::IdentityMismatch);
                }
                join(None, &file.0)?
            }
            RootRelativeDirectory::Descendant(route) => {
                let directory = native::metadata_directory(&self.directory, &route.0)?;
                if FileIdentity::from_file(directory)?
                    != FileIdentity::from_file(output.try_clone()?.into_std_file())?
                {
                    return Err(RootReadError::IdentityMismatch);
                }
                join(Some(&route.0), &file.0)?
            }
        };
        // Pin from the supplied output capability, then acquire DATA from the
        // selected root. The two identities are compared before the first Read.
        let expected_file = native::metadata_file(output, &file.0)?;
        if !native::regular(&expected_file)? {
            return Err(RootReadError::NonRegular);
        }
        let expected = FileIdentity::from_file(expected_file)?;
        Ok(ReadOnlyRootPlan {
            scope: self,
            relative,
            expected,
            #[cfg(test)]
            before_data_acquire: None,
            #[cfg(test)]
            after_data_acquire: None,
            _hooks: std::marker::PhantomData,
        })
    }
    /// Acquire once through the genuine selected root with no generic fallback.
    /// Failed acquisition remains an actual content attempt; no eager read occurs.
    pub fn open_read_only_root<'scope>(
        &'scope self,
        plan: &ReadOnlyRootPlan<'_, '_>,
    ) -> Result<RootAcquiredRead<'scope>, RootReadError> {
        if !std::ptr::eq(self, plan.scope) {
            return Err(RootReadError::ForeignScope);
        }
        if !native::supported() {
            return Err(RootReadError::UnsupportedPlatform);
        }
        #[cfg(test)]
        if let Some(before) = plan.before_data_acquire {
            before();
        }
        let mut attempt = Attempt::begin(self);
        let acquired = native::data_file(&self.directory, &plan.relative)?;
        #[cfg(test)]
        if let Some(after) = plan.after_data_acquire {
            after();
        }
        let regular = native::regular(&acquired);
        let file = cap_std::fs::File::from_std(acquired);
        // Even a mismatched opened target must remain visible to its preread
        // witness. Refusal is incomplete and never yields a reader.
        attempt.opened(&file, Some(ReadClass::WorkspaceContent));
        let validated = (|| {
            if !regular? {
                return Err(RootReadError::NonRegular);
            }
            let actual = FileIdentity::from_file(file.try_clone()?.into_std())?;
            if actual != plan.expected {
                return Err(RootReadError::IdentityMismatch);
            }
            Ok(())
        })();
        if validated.is_err() {
            self.shared
                .state()
                .fault
                .get_or_insert(super::ObservationError::FileIdentity);
        }
        validated?;
        Ok(ObservedContentFile { file, attempt })
    }
}
fn join(directory: Option<&Path>, file: &Path) -> Result<PathBuf, RootReadError> {
    let mut path = PathBuf::new();
    let bytes = directory
        .map_or(0, |p| p.as_os_str().len())
        .checked_add(file.as_os_str().len())
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| io::Error::other("root path capacity"))?;
    path.try_reserve_exact(bytes).map_err(io::Error::other)?;
    if let Some(directory) = directory {
        path.push(directory);
    }
    path.push(file);
    Ok(path)
}
