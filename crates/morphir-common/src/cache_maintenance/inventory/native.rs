use super::is_link_like;
#[cfg(unix)]
use cap_fs_ext::OpenOptionsSyncExt;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, DirEntry, Metadata, OpenOptions};
use same_file::Handle;
use std::ffi::OsStr;
use std::io;

pub(super) fn spelling_matches(
    directory: &Dir,
    siblings: &[DirEntry],
    child: &DirEntry,
    metadata: &Metadata,
    observed_identity: &str,
    registered_path: &str,
) -> bool {
    let depth = observed_identity.split('/').count();
    let registered_prefix = registered_path
        .split('/')
        .take(depth)
        .collect::<Vec<_>>()
        .join("/");
    if registered_prefix == observed_identity {
        return true;
    }
    let Some(registered_component) = registered_prefix.rsplit('/').next() else {
        return false;
    };
    if siblings.iter().any(|sibling| {
        sibling.file_name() == OsStr::new(registered_component)
            && sibling.file_name() != child.file_name()
    }) {
        return false;
    }
    same_object(
        directory,
        OsStr::new(registered_component),
        child.file_name().as_os_str(),
        metadata,
    )
}

fn same_object(
    directory: &Dir,
    registered_name: &OsStr,
    observed_name: &OsStr,
    observed_metadata: &Metadata,
) -> bool {
    if !ordinary(observed_metadata) {
        return false;
    }
    let Ok(registered_metadata) = directory.symlink_metadata(registered_name) else {
        return false;
    };
    if !ordinary(&registered_metadata)
        || registered_metadata.is_dir() != observed_metadata.is_dir()
        || registered_metadata.is_file() != observed_metadata.is_file()
    {
        return false;
    }

    let registered = object_handle(directory, registered_name, &registered_metadata);
    let observed = object_handle(directory, observed_name, observed_metadata);
    matches!((registered, observed), (Ok(left), Ok(right)) if left == right)
}

fn ordinary(metadata: &Metadata) -> bool {
    !is_link_like(metadata) && (metadata.is_dir() || metadata.is_file())
}

fn object_handle(directory: &Dir, name: &OsStr, expected: &Metadata) -> io::Result<Handle> {
    let file = if expected.is_dir() {
        directory.open_dir_nofollow(name)?.into_std_file()
    } else {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        options.nonblock(true);
        directory.open_with(name, &options)?.into_std()
    };
    let opened = file.metadata()?;
    if opened.is_dir() != expected.is_dir() || opened.is_file() != expected.is_file() {
        return Err(io::Error::other("cache object type changed while opening"));
    }
    Handle::from_file(file)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::os::unix::net::UnixListener;
    use tempfile::TempDir;

    #[test]
    fn stale_regular_metadata_cannot_block_on_a_replacement_socket() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("artifact");
        std::fs::write(&path, b"ordinary").unwrap();
        let directory = Dir::open_ambient_dir(root.path(), ambient_authority()).unwrap();
        let stale = directory.symlink_metadata("artifact").unwrap();
        std::fs::remove_file(&path).unwrap();
        let _listener = UnixListener::bind(&path).unwrap();

        assert!(object_handle(&directory, OsStr::new("artifact"), &stale).is_err());
    }
}
