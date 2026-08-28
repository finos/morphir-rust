use std::io::{Read, Write};

use morphir_common::vfs::{
    ManifestLastPublisher, PhysicalPublisher, PublicationCapabilities, Publisher, memory_root,
    physical_root,
};
use vfs::VfsPath;

fn assert_bounded_vfs_contract(root: VfsPath) {
    let source_dir = root.join("source").unwrap();
    source_dir.create_dir_all().unwrap();
    let source = source_dir.join("large.json").unwrap();
    let bytes = vec![b'x'; 32 * 1024];

    let mut writer = source.create_file().unwrap();
    writer.write_all(&bytes).unwrap();
    writer.flush().unwrap();
    drop(writer);

    let mut reader = source.open_file().unwrap();
    let mut observed = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = reader.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        observed.extend_from_slice(&chunk[..read]);
    }
    assert_eq!(observed, bytes);

    let moved = source_dir.join("moved.json").unwrap();
    source.move_file(&moved).unwrap();
    assert!(!source.exists().unwrap());
    assert!(moved.exists().unwrap());

    let entries = root
        .walk_dir()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(entries.iter().any(|path| path == &moved));

    let destination = root.join("destination").unwrap();
    source_dir.move_dir(&destination).unwrap();
    assert!(destination.join("moved.json").unwrap().exists().unwrap());
}

#[test]
fn memory_backend_satisfies_the_bounded_contract() {
    assert_bounded_vfs_contract(memory_root());
}

#[test]
fn physical_backend_satisfies_the_bounded_contract() {
    let temporary = tempfile::tempdir().unwrap();
    assert_bounded_vfs_contract(physical_root(temporary.path()));
}

#[test]
fn publication_capabilities_are_not_inferred_from_generic_move_support() {
    assert_eq!(
        PhysicalPublisher.capabilities(),
        PublicationCapabilities {
            atomic_file_replace: true,
            atomic_dir_replace: true,
        }
    );
    assert_eq!(
        ManifestLastPublisher.capabilities(),
        PublicationCapabilities {
            atomic_file_replace: false,
            atomic_dir_replace: false,
        }
    );
}
