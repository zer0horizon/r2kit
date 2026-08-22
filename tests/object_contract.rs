use aws_sdk_s3::{config::Credentials, primitives::ByteStream};
use r2kit::{Error, R2Client, R2Config, ValidationError};

fn offline_bucket() -> r2kit::Bucket {
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("offline-access")
        .secret_access_key("offline-secret")
        .build()
        .unwrap();
    let sdk_config = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .endpoint_url(config.endpoint_url())
        .region(aws_sdk_s3::config::Region::new("auto"))
        .credentials_provider(Credentials::new(
            "offline-access",
            "offline-secret",
            None,
            None,
            "contract-test",
        ))
        .build();
    R2Client::from_sdk(aws_sdk_s3::Client::from_conf(sdk_config))
        .bucket("contract-tests")
        .unwrap()
}

#[test]
fn enforces_the_documented_r2_bucket_name_length() {
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("offline-access")
        .secret_access_key("offline-secret")
        .build()
        .unwrap();
    let client = R2Client::new(config);

    assert!(client.bucket("a".repeat(63)).is_ok());
    assert!(matches!(
        client.bucket("a".repeat(64)),
        Err(Error::InvalidInput {
            field: "bucket",
            ..
        })
    ));
}

#[tokio::test]
async fn rejects_invalid_list_options_before_network() {
    let bucket = offline_bucket();

    let zero = bucket.list().limit(0).send().await.unwrap_err();
    assert!(matches!(
        zero,
        Error::Validation(ValidationError::ListLimitOutOfRange {
            provided: 0,
            min: 1,
            max: 1_000
        })
    ));

    let too_large = bucket.list().limit(1_001).send().await.unwrap_err();
    assert!(matches!(
        too_large,
        Error::Validation(ValidationError::ListLimitOutOfRange {
            provided: 1_001,
            min: 1,
            max: 1_000
        })
    ));

    let empty_token = bucket
        .list()
        .continuation_token("")
        .send()
        .await
        .unwrap_err();
    assert!(matches!(
        empty_token,
        Error::InvalidInput {
            field: "continuation_token",
            ..
        }
    ));
}

#[tokio::test]
async fn rejects_invalid_object_writes_before_network() {
    let bucket = offline_bucket();

    let empty_key = bucket.put_bytes("", Vec::new()).await.unwrap_err();
    assert!(matches!(
        empty_key,
        Error::InvalidInput { field: "key", .. }
    ));

    let too_large = bucket
        .put_stream(
            "large.bin",
            ByteStream::from_static(&[]),
            5 * 1024 * 1024 * 1024 + 1,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        too_large,
        Error::Validation(ValidationError::SingleUploadTooLarge {
            provided: 5_368_709_121,
            max: 5_368_709_120
        })
    ));
}
