use std::time::Duration;

use r2kit::{MultipartSessionSnapshot, PartNumber, R2Client, R2Config};

fn offline_bucket() -> r2kit::Bucket {
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("contract-access-key")
        .secret_access_key("contract-secret-key")
        .session_token("contract-session-token")
        .build()
        .unwrap();
    R2Client::new(config).bucket("r2kit").unwrap()
}

#[tokio::test]
async fn presigns_upload_part_without_exposing_secrets_in_debug() {
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("contract-access-key")
        .secret_access_key("contract-secret-key")
        .session_token("contract-session-token")
        .build()
        .unwrap();
    let client = R2Client::new(config);
    let client_debug = format!("{client:?}");
    assert!(!client_debug.contains("contract-access-key"));
    assert!(!client_debug.contains("contract-secret-key"));
    assert!(!client_debug.contains("contract-session-token"));
    let bucket = client.bucket("r2kit").unwrap();
    let snapshot = MultipartSessionSnapshot::restore(
        "r2kit",
        "_r2kit-tests/offline/file.bin",
        "offline-upload-id",
        11 * 1024 * 1024,
        5 * 1024 * 1024,
    )
    .unwrap();
    let session = bucket.resume_presigned_multipart(snapshot).unwrap();
    let part = session
        .presign_part(PartNumber::try_from(2).unwrap(), Duration::from_secs(900))
        .await
        .unwrap();

    assert_eq!(part.request().method(), "PUT");
    assert_eq!(part.content_length(), 5 * 1024 * 1024);
    let exposed = part.request().url().expose();
    assert!(exposed.contains("partNumber=2"));
    assert!(exposed.contains("uploadId=offline-upload-id"));
    assert!(exposed.contains("X-Amz-Signature="));

    let debug = format!("{part:?}");
    assert!(!debug.contains("X-Amz-Signature"));
    assert!(!debug.contains("contract-access-key"));
    assert!(!debug.contains("contract-session-token"));
}

#[tokio::test]
async fn rejects_invalid_presign_expiry_before_network() {
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("access")
        .secret_access_key("secret")
        .build()
        .unwrap();
    let bucket = R2Client::new(config).bucket("r2kit").unwrap();
    let snapshot = MultipartSessionSnapshot::restore(
        "r2kit",
        "key",
        "upload-id",
        5 * 1024 * 1024,
        5 * 1024 * 1024,
    )
    .unwrap();
    let session = bucket.resume_presigned_multipart(snapshot).unwrap();
    let part = PartNumber::try_from(1).unwrap();

    assert!(session.presign_part(part, Duration::ZERO).await.is_err());
    assert!(
        session
            .presign_part(part, Duration::from_secs(604_801))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn presigns_single_get_and_put_with_redacted_bearer_urls() {
    let bucket = offline_bucket();
    let get = bucket
        .presign_get("objects/file.bin", Duration::from_secs(900))
        .await
        .unwrap();
    assert_eq!(get.method(), "GET");
    assert!(get.url().expose().contains("X-Amz-Signature="));
    assert!(!format!("{get:?}").contains("X-Amz-Signature"));

    let put = bucket
        .presign_put("objects/file.bin", 42, Duration::from_secs(900))
        .await
        .unwrap();
    assert_eq!(put.content_length(), 42);
    assert_eq!(put.request().method(), "PUT");
    assert!(put.request().url().expose().contains("X-Amz-Signature="));
    assert!(!format!("{put:?}").contains("X-Amz-Signature"));
}

#[tokio::test]
async fn rejects_invalid_single_object_presign_contracts() {
    let bucket = offline_bucket();

    assert!(
        bucket
            .presign_get("key", Duration::from_millis(999))
            .await
            .is_err()
    );
    assert!(
        bucket
            .presign_put("key", 5 * 1024 * 1024 * 1024 + 1, Duration::from_secs(60))
            .await
            .is_err()
    );
}
