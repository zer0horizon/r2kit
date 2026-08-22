use r2kit::{ConfigError, R2Config};

#[test]
fn builds_r2_endpoint_and_auto_region() {
    let config = R2Config::builder()
        .account_id("abc123")
        .access_key_id("access")
        .secret_access_key("secret")
        .build()
        .expect("configuration should be valid");

    assert_eq!(config.region(), "auto");
    assert_eq!(
        config.endpoint_url(),
        "https://abc123.r2.cloudflarestorage.com"
    );
}

#[test]
fn redacts_all_credentials_from_debug() {
    let config = R2Config::builder()
        .account_id("abc123")
        .access_key_id("visible-access-key")
        .secret_access_key("visible-secret-key")
        .session_token("visible-session-token")
        .build()
        .unwrap();
    let output = format!("{config:?}");
    assert!(!output.contains("visible-access-key"));
    assert!(!output.contains("visible-secret-key"));
    assert!(!output.contains("visible-session-token"));
}

#[test]
fn rejects_an_invalid_account_id() {
    let error = R2Config::builder()
        .account_id("has whitespace")
        .access_key_id("access")
        .secret_access_key("secret")
        .build()
        .expect_err("account IDs cannot contain whitespace");

    assert_eq!(error, ConfigError::InvalidAccountId);
}
