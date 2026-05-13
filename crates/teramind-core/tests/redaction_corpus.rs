use teramind_core::redact::Redactor;

#[test]
fn aws_access_key_is_redacted() {
    let r = Redactor::with_default_rules();
    let input = "key=AKIAIOSFODNN7EXAMPLE next";
    let out = r.apply(input);
    assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "raw key leaked: {out}");
    assert!(out.contains("«redacted:aws_access_key»"));
}
