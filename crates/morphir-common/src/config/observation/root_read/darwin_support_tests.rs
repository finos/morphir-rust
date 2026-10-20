//! Source-first support controls: this is native metadata use, never content I/O.
use super::*;
#[test]
fn actual_root_beneath_support_probe_does_not_issue_content_observations() {
    let fixture = tempfile::tempdir().unwrap();
    let root = Dir::open_ambient_dir(fixture.path(), cap_std::ambient_authority()).unwrap();
    let scope = super::super::super::SourceReadScope::bind_workspace(
        &root,
        fixture.path(),
        super::super::super::ReadLimits::new(4, 4, 1024, 0).unwrap(),
    )
    .unwrap();
    ensure_support(&root).unwrap();
    let snapshot = scope.observation().snapshot();
    assert_eq!(snapshot.total_counts().content_attempts(), 0);
    assert_eq!(snapshot.total_counts().read_calls(), 0);
    assert_eq!(snapshot.total_counts().returned_bytes(), 0);
    assert_eq!(
        snapshot.coverage(),
        super::super::super::Coverage::Unobserved
    );
}
#[test]
fn only_the_pinned_beneath_escape_errno_attests_support() {
    assert!(probe_refusal(libc::ENOTCAPABLE).is_ok());
    for error in [
        libc::EXDEV,
        libc::EACCES,
        libc::EPERM,
        libc::EINVAL,
        libc::ENOENT,
        libc::EIO,
        libc::ELOOP,
        libc::ENOSYS,
    ] {
        assert!(matches!(
            probe_refusal(error),
            Err(RootReadError::UnsupportedPlatform)
        ));
    }
}
