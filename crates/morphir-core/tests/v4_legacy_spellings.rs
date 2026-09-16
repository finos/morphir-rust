use morphir_core::ir::DiagnosticCode;
use morphir_core::ir::v4::{SpellingMode, accept_member, with_spelling_mode};

#[test]
fn current_mode_accepts_a_legacy_member_with_a_warning() {
    let (result, warnings) = with_spelling_mode(SpellingMode::Current, || {
        accept_member("IfThenElse", "thenBranch", "/IfThenElse/thenBranch")
    });
    assert_eq!(result.unwrap(), "then");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, DiagnosticCode::LegacySpelling);
    assert_eq!(warnings[0].cursor, "/IfThenElse/thenBranch");
}

#[test]
fn pinned_mode_refuses_a_legacy_member() {
    let (result, warnings) = with_spelling_mode(SpellingMode::Pinned, || {
        accept_member("Function", "arg", "/Function/arg")
    });
    assert_eq!(result.unwrap_err().code, DiagnosticCode::UnknownMember);
    assert!(warnings.is_empty());
}

#[test]
fn a_canonical_member_is_accepted_silently_in_both_modes() {
    for mode in [SpellingMode::Current, SpellingMode::Pinned] {
        let (result, warnings) = with_spelling_mode(mode, || {
            accept_member("Function", "parameterType", "/Function/parameterType")
        });
        assert_eq!(result.unwrap(), "parameterType");
        assert!(warnings.is_empty());
    }
}
