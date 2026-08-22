use r2kit::{Error, ManagedUploadCancellation, MultipartSessionSnapshot, R2Client, R2Config};

fn offline_bucket() -> r2kit::Bucket {
    let config = R2Config::builder()
        .account_id("0123456789abcdef0123456789abcdef")
        .access_key_id("managed-access")
        .secret_access_key("managed-secret")
        .build()
        .unwrap();
    R2Client::new(config).bucket("managed-tests").unwrap()
}

#[tokio::test]
async fn rejects_invalid_managed_limits_before_file_or_network_io() {
    let bucket = offline_bucket();

    let concurrency = bucket
        .managed_multipart("object.bin")
        .unwrap()
        .concurrency(0)
        .upload_file("path-does-not-exist")
        .await
        .unwrap_err();
    assert!(matches!(
        concurrency.error(),
        Error::InvalidInput {
            field: "concurrency",
            ..
        }
    ));

    let attempts = bucket
        .managed_multipart("object.bin")
        .unwrap()
        .max_attempts(11)
        .upload_file("path-does-not-exist")
        .await
        .unwrap_err();
    assert!(matches!(
        attempts.error(),
        Error::InvalidInput {
            field: "max_attempts",
            ..
        }
    ));

    let part_size = bucket
        .managed_multipart("object.bin")
        .unwrap()
        .part_size(1024)
        .upload_file("path-does-not-exist")
        .await
        .unwrap_err();
    assert!(matches!(
        part_size.error(),
        Error::InvalidInput {
            field: "part_size",
            ..
        }
    ));
}

#[test]
fn resumed_builder_redacts_the_upload_id() {
    let bucket = offline_bucket();
    let snapshot = MultipartSessionSnapshot::restore(
        "managed-tests",
        "object.bin",
        "sensitive-managed-upload-id",
        11 * 1024 * 1024,
        5 * 1024 * 1024,
    )
    .unwrap();
    let builder = bucket.resume_managed_multipart(snapshot).unwrap();
    let debug = format!("{builder:?}");
    assert!(!debug.contains("sensitive-managed-upload-id"));
}

#[tokio::test]
async fn pre_cancelled_upload_never_starts_a_remote_session() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cancelled.bin");
    tokio::fs::write(&path, vec![0_u8; 5 * 1024 * 1024])
        .await
        .unwrap();
    let cancellation = ManagedUploadCancellation::new();
    cancellation.cancel();

    let error = offline_bucket()
        .managed_multipart("cancelled.bin")
        .unwrap()
        .cancellation_token(cancellation)
        .upload_file(path)
        .await
        .unwrap_err();

    assert!(matches!(error.error(), Error::Cancelled));
    assert!(!error.was_aborted());
    assert!(error.snapshot().is_none());
}
