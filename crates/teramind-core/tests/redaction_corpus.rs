use teramind_core::redact::Redactor;

#[test]
fn aws_access_key_is_redacted() {
    let r = Redactor::with_default_rules();
    let input = "key=AKIAIOSFODNN7EXAMPLE next";
    let out = r.apply(input);
    assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "raw key leaked: {out}");
    assert!(out.contains("«redacted:aws_access_key»"));
}

#[test]
fn github_pat_is_redacted() {
    let r = Redactor::with_default_rules();
    for sample in ["ghp_1234567890abcdefghijklmnopqrstuvwxyz",
                   "gho_abcdefghijklmnopqrstuvwxyz1234567890",
                   "ghs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                   "ghr_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"] {
        let out = r.apply(sample);
        assert!(!out.contains(sample), "leaked: {sample} -> {out}");
        assert!(out.contains("«redacted:github_token»"));
    }
}
