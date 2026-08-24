use std::time::{Duration, UNIX_EPOCH};
use std::{env, time::SystemTime};

use futures_util::TryStreamExt;
use r2kit::{CacheControl, Error, ObjectUploadOptions, R2Client, R2Config, mime};

fn live_client() -> R2Client {
    assert_eq!(env::var("R2KIT_LIVE_TESTS").as_deref(), Ok("1"));
    assert_eq!(
        env::var("R2KIT_LIVE_BUCKET").as_deref(),
        Ok("r2kit-live-tests")
    );
    R2Client::new(R2Config::from_env().expect("R2 live credentials are required"))
}

#[tokio::test]
#[ignore = "requires explicit bucket-scoped R2 credentials"]
async fn live_bucket_preflight_confirms_read_access() {
    live_client()
        .validate_bucket("r2kit-live-tests")
        .await
        .expect("dedicated live bucket must be accessible");
}

#[tokio::test]
#[ignore = "requires explicit bucket-scoped R2 credentials"]
async fn live_core_object_round_trip_and_pagination() {
    let client = live_client();
    let bucket = client.bucket("r2kit-live-tests").unwrap();
    let prefix = format!("_r2kit-tests/{}/objects/", uuid::Uuid::new_v4());
    let first_key = format!("{prefix}a.txt");
    let second_key = format!("{prefix}b.txt");
    let first_body = b"r2kit object API: first".to_vec();
    let second_body = vec![
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x2b, 0x32, 0xca, 0xce, 0x2c,
        0x51, 0xc8, 0x4f, 0xca, 0x4a, 0x4d, 0x2e, 0x51, 0x70, 0x0c, 0xf0, 0xb4, 0x52, 0x28, 0x4e,
        0x4d, 0xce, 0xcf, 0x4b, 0x01, 0x00, 0xe2, 0x09, 0xb2, 0x17, 0x18, 0x00, 0x00, 0x00,
    ];
    let expires = UNIX_EPOCH + Duration::from_secs(1_893_456_000);

    let result = async {
        let options = ObjectUploadOptions::builder()
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .content_disposition("attachment; filename=a.txt")
            .content_language("en-US, vi")
            .expires(expires)
            .custom_metadata("test-run", "object-round-trip")
            .custom_metadata("tenant-id", "tenant-42")
            .build();
        let put = bucket
            .put_bytes_with_options(&first_key, first_body.clone(), options)
            .await
            .map_err(|_| "first put failed")?;
        let encoded_options = ObjectUploadOptions::builder()
            .content_type(mime::TEXT_PLAIN_UTF_8)
            .content_encoding("gzip")
            .build();
        bucket
            .put_bytes_with_options(&second_key, second_body.clone(), encoded_options)
            .await
            .map_err(|_| "second put failed")?;

        let metadata = bucket.head(&first_key).await.map_err(|_| "head failed")?;
        if metadata.size() != first_body.len() as u64
            || metadata.etag() != put.etag()
            || metadata.content_disposition() != Some("attachment; filename=a.txt")
            || metadata.content_language() != Some("en-US, vi")
            || metadata.expires() != Some(expires)
            || metadata.custom().get("test-run").map(String::as_str) != Some("object-round-trip")
            || metadata.custom().get("tenant-id").map(String::as_str) != Some("tenant-42")
        {
            return Err("head metadata differs from put result");
        }

        let download = bucket.get(&first_key).await.map_err(|_| "get failed")?;
        if download.metadata().size() != first_body.len() as u64 {
            return Err("download metadata has wrong size");
        }
        let actual = download
            .into_body()
            .collect()
            .await
            .map_err(|_| "download body failed")?
            .into_bytes();
        if actual.as_ref() != first_body {
            return Err("downloaded bytes differ");
        }

        let encoded_metadata = bucket
            .head(&second_key)
            .await
            .map_err(|_| "encoded object head failed")?;
        if encoded_metadata.content_encoding() != Some("gzip") {
            return Err("encoded object metadata differs");
        }
        let encoded = bucket
            .get(&second_key)
            .await
            .map_err(|_| "encoded object get failed")?
            .into_body()
            .collect()
            .await
            .map_err(|_| "encoded object body failed")?
            .into_bytes();
        if encoded.as_ref() != second_body {
            return Err("encoded object bytes differ");
        }

        let pages: Vec<_> = bucket
            .list()
            .prefix(&prefix)
            .limit(1)
            .into_pages()
            .try_collect()
            .await
            .map_err(|_| "page stream failed")?;
        if pages.len() != 2 || pages.iter().any(|page| page.objects().len() != 1) {
            return Err("page stream must return two one-object pages");
        }

        let deleted = bucket
            .delete_objects([&first_key, &second_key])
            .await
            .map_err(|_| "batch delete request failed")?;
        if !deleted.is_complete() || deleted.deleted_keys().len() != 2 {
            return Err("batch delete did not report both keys as deleted");
        }
        if !matches!(bucket.head(&first_key).await, Err(Error::NotFound)) {
            return Err("batch-deleted key is still readable");
        }
        Ok::<(), &'static str>(())
    }
    .await;

    let _ = bucket.delete(&first_key).await;
    let _ = bucket.delete(&second_key).await;
    result.unwrap();
}

#[tokio::test]
#[ignore = "requires explicit bucket-scoped R2 credentials"]
async fn live_presigned_put_and_get_round_trip() {
    let client = live_client();
    let bucket = client.bucket("r2kit-live-tests").unwrap();
    let key = format!("_r2kit-tests/{}/presigned.bin", uuid::Uuid::new_v4());
    let body = b"r2kit presigned object contract".to_vec();
    let expires: SystemTime = UNIX_EPOCH + Duration::from_secs(1_893_456_000);

    let result = async {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "failed to build HTTP client")?;
        let options = ObjectUploadOptions::builder()
            .content_type(mime::IMAGE_JPEG)
            .cache_control(
                CacheControl::new()
                    .with_public()
                    .with_max_age(Duration::from_secs(3_600)),
            )
            .content_disposition("attachment; filename=presigned.bin")
            .content_language("en-US")
            .expires(expires)
            .custom_metadata("upload-mode", "presigned")
            .build();
        let put = bucket
            .presign_put_with_options(&key, body.len() as u64, Duration::from_secs(900), options)
            .await
            .map_err(|_| "PUT presign failed")?;
        let (method, url, headers) = put.into_request().into_exposed_parts();
        let method = method.parse().map_err(|_| "invalid PUT method")?;
        let mut request = http.request(method, url).body(body.clone());
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.send().await.map_err(|_| "PUT transport failed")?;
        if !response.status().is_success() {
            return Err("R2 rejected presigned PUT");
        }

        let get = bucket
            .presign_get(&key, Duration::from_secs(900))
            .await
            .map_err(|_| "GET presign failed")?;
        let (method, url, headers) = get.into_exposed_parts();
        let method = method.parse().map_err(|_| "invalid GET method")?;
        let mut request = http.request(method, url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.send().await.map_err(|_| "GET transport failed")?;
        if !response.status().is_success() {
            return Err("R2 rejected presigned GET");
        }
        let actual = response.bytes().await.map_err(|_| "GET body failed")?;
        if actual.as_ref() != body {
            return Err("presigned GET bytes differ");
        }
        let metadata = bucket.head(&key).await.map_err(|_| "HEAD failed")?;
        if metadata.content_type() != Some("image/jpeg")
            || metadata
                .cache_control()
                .is_none_or(|value| !value.contains("public") || !value.contains("max-age=3600"))
            || metadata.content_disposition() != Some("attachment; filename=presigned.bin")
            || metadata.content_language() != Some("en-US")
            || metadata.expires() != Some(expires)
            || metadata.custom().get("upload-mode").map(String::as_str) != Some("presigned")
        {
            return Err("typed object metadata was not persisted");
        }
        Ok::<(), &'static str>(())
    }
    .await;

    let _ = bucket.delete(&key).await;
    result.unwrap();
}
