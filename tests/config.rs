use std::{str::FromStr, time::Duration};

use r2kit::{ConfigError, R2Client, R2Config, R2Jurisdiction};

#[test]
fn builds_r2_endpoint_and_auto_region() {
    let account_id = "0123456789abcdef0123456789abcdef";
    let config = R2Config::builder()
        .account_id(account_id)
        .access_key_id("access")
        .secret_access_key("secret")
        .build()
        .expect("configuration should be valid");

    assert_eq!(config.region(), "auto");
    assert_eq!(
        config.endpoint_url(),
        format!("https://{account_id}.r2.cloudflarestorage.com")
    );
}

#[test]
fn redacts_all_credentials_from_debug() {
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
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

#[test]
fn rejects_a_non_hex_account_id() {
    let result = R2Config::builder()
        .account_id("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")
        .access_key_id("access")
        .secret_access_key("secret")
        .build();

    assert_eq!(result.unwrap_err(), ConfigError::InvalidAccountId);
}

#[test]
fn builds_every_supported_jurisdiction_endpoint() {
    let account_id = "0123456789abcdef0123456789abcdef";
    let cases = [
        (
            R2Jurisdiction::Default,
            format!("https://{account_id}.r2.cloudflarestorage.com"),
        ),
        (
            R2Jurisdiction::Eu,
            format!("https://{account_id}.eu.r2.cloudflarestorage.com"),
        ),
        (
            R2Jurisdiction::Us,
            format!("https://{account_id}.us.r2.cloudflarestorage.com"),
        ),
        (
            R2Jurisdiction::FedRamp,
            format!("https://{account_id}.fedramp.r2.cloudflarestorage.com"),
        ),
    ];

    for (jurisdiction, endpoint) in cases {
        let config = R2Config::builder()
            .account_id(account_id)
            .access_key_id("access")
            .secret_access_key("secret")
            .jurisdiction(jurisdiction)
            .build()
            .unwrap();

        assert_eq!(config.jurisdiction(), jurisdiction);
        assert_eq!(config.endpoint_url(), endpoint);
    }
}

#[test]
fn parses_only_documented_jurisdictions() {
    assert_eq!(
        R2Jurisdiction::from_str("default").unwrap(),
        R2Jurisdiction::Default
    );
    assert_eq!(
        R2Jurisdiction::from_str("fedramp").unwrap(),
        R2Jurisdiction::FedRamp
    );
    assert_eq!(
        R2Jurisdiction::from_str("APAC").unwrap_err(),
        ConfigError::InvalidJurisdiction
    );
}

#[test]
fn applies_validated_timeouts_and_sdk_retries() {
    let connect = Duration::from_secs(3);
    let read = Duration::from_secs(15);
    let operation = Duration::from_secs(60);
    let attempt = Duration::from_secs(20);
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("access")
        .secret_access_key("secret")
        .connect_timeout(connect)
        .read_timeout(read)
        .operation_timeout(operation)
        .operation_attempt_timeout(attempt)
        .sdk_max_attempts(4)
        .build()
        .unwrap();

    let client = R2Client::new(config);
    let sdk_config = client.as_sdk().config();
    let timeouts = sdk_config.timeout_config().unwrap();
    assert_eq!(timeouts.connect_timeout(), Some(connect));
    assert_eq!(timeouts.read_timeout(), Some(read));
    assert_eq!(timeouts.operation_timeout(), Some(operation));
    assert_eq!(timeouts.operation_attempt_timeout(), Some(attempt));
    assert_eq!(sdk_config.retry_config().unwrap().max_attempts(), 4);
}

#[test]
fn preserves_sdk_defaults_when_transport_options_are_absent() {
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("access")
        .secret_access_key("secret")
        .build()
        .unwrap();
    let client = R2Client::new(config);

    assert!(client.as_sdk().config().timeout_config().is_none());
    assert!(client.as_sdk().config().retry_config().is_none());
}

#[test]
fn rejects_zero_timeouts_and_attempt_limits() {
    for error in [
        config_builder()
            .connect_timeout(Duration::ZERO)
            .build()
            .unwrap_err(),
        config_builder()
            .read_timeout(Duration::ZERO)
            .build()
            .unwrap_err(),
        config_builder()
            .operation_timeout(Duration::ZERO)
            .build()
            .unwrap_err(),
        config_builder()
            .operation_attempt_timeout(Duration::ZERO)
            .build()
            .unwrap_err(),
    ] {
        assert!(matches!(error, ConfigError::InvalidTimeout(_)));
    }

    assert_eq!(
        config_builder().sdk_max_attempts(0).build().unwrap_err(),
        ConfigError::InvalidAttempts("sdk_max_attempts")
    );
}

#[test]
fn rejects_an_attempt_timeout_longer_than_the_operation() {
    let error = config_builder()
        .operation_timeout(Duration::from_secs(5))
        .operation_attempt_timeout(Duration::from_secs(6))
        .build()
        .unwrap_err();

    assert_eq!(error, ConfigError::InconsistentTimeouts);
}

#[test]
fn builder_debug_redacts_credentials_before_build() {
    let builder = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("builder-access")
        .secret_access_key("builder-secret")
        .session_token("builder-session");
    let output = format!("{builder:?}");

    assert!(!output.contains("builder-access"));
    assert!(!output.contains("builder-secret"));
    assert!(!output.contains("builder-session"));
}

fn config_builder() -> r2kit::R2ConfigBuilder {
    R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("access")
        .secret_access_key("secret")
}
