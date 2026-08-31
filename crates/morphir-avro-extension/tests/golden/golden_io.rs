use super::*;
use pretty_assertions::assert_eq;

#[test]
fn golden_update_mode_requires_exact_one_and_refuses_ci() {
    assert_eq!(golden_update_mode(None, None), Ok(false));
    assert_eq!(golden_update_mode(Some(OsStr::new("0")), None), Ok(false));
    assert_eq!(
        golden_update_mode(Some(OsStr::new("false")), None),
        Ok(false)
    );
    assert_eq!(golden_update_mode(Some(OsStr::new("1")), None), Ok(true));
    assert_eq!(
        golden_update_mode(Some(OsStr::new("1")), Some(OsStr::new("true"))),
        Err("refusing to update goldens in CI")
    );
}
