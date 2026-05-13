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

#[test]
fn slack_token_is_redacted() {
    let r = Redactor::with_default_rules();
    let s = "xoxb-1234567890-1234567890-aBcDeFgHiJkLmNoPqRsTuVwX";
    let out = r.apply(s);
    assert!(!out.contains(s));
    assert!(out.contains("«redacted:slack_token»"));
}

#[test]
fn jwt_is_redacted() {
    let r = Redactor::with_default_rules();
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1MSJ9.ZmFrZXNpZ25hdHVyZQ";
    let out = r.apply(jwt);
    assert!(!out.contains(jwt));
    assert!(out.contains("«redacted:jwt»"));
}

#[test]
fn pem_private_key_is_redacted() {
    let r = Redactor::with_default_rules();
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj...==\n-----END RSA PRIVATE KEY-----";
    let out = r.apply(pem);
    assert!(!out.contains("MIIBOgIBAAJBAKj"));
    assert!(out.contains("«redacted:pem_private_key»"));
}

#[test]
fn password_kv_is_redacted() {
    let r = Redactor::with_default_rules();
    for s in ["password=hunter2 next", "PWD=correcthorsebatterystaple "] {
        let out = r.apply(s);
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("correcthorsebatterystaple"));
    }
}

#[test]
fn env_key_allowlist_is_redacted() {
    let r = Redactor::with_default_rules();
    for s in ["API_SECRET=abcdef ", "MY_TOKEN=xyz123 ", "DB_PASSWORD=p", "FOO_CREDENTIAL=bar", "X_KEY=val"] {
        let out = r.apply(s);
        let val = s.split('=').nth(1).unwrap().split_whitespace().next().unwrap();
        assert!(!out.contains(val), "leaked: {s} -> {out}");
    }
}
