use std::{env, time::Duration};

use r2kit::{CompletionManifest, PartNumber, R2Client, R2Config, UploadedPart};

const MIB: usize = 1024 * 1024;

fn live_client() -> R2Client {
    assert_eq!(env::var("R2KIT_LIVE_TESTS").as_deref(), Ok("1"));
    assert_eq!(
        env::var("R2KIT_LIVE_BUCKET").as_deref(),
        Ok("r2kit-live-tests")
    );

    let config = R2Config::from_env().expect("R2 live credentials are required");
    R2Client::new(config)
}

#[tokio::test]
#[ignore = "requires explicit bucket-scoped R2 credentials"]
async fn live_presigned_multipart_round_trip() {
    let client = live_client();
    let bucket = client.bucket("r2kit-live-tests").unwrap();
    let key = format!("_r2kit-tests/{}/complete.bin", uuid::Uuid::new_v4());
    let chunks = [vec![0x11; 5 * MIB], vec![0x22; 5 * MIB], vec![0x33; MIB]];
    let session = bucket
        .presigned_multipart(&key)
        .unwrap()
        .file_size((11 * MIB) as u64)
        .part_size((5 * MIB) as u64)
        .create()
        .await
        .unwrap();

    let result = async {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "failed to build HTTP client")?;
        let mut uploaded = Vec::new();
        for (index, body) in chunks.iter().enumerate() {
            let number =
                PartNumber::try_from((index + 1) as u16).map_err(|_| "invalid part number")?;
            let signed = session
                .presign_part(number, Duration::from_secs(900))
                .await
                .map_err(|_| "presign failed")?;
            let (method, url, headers) = signed.into_request().into_exposed_parts();
            let method = method.parse().map_err(|_| "invalid signed method")?;
            let mut request = http.request(method, url).body(body.clone());
            for (name, value) in headers {
                request = request.header(name, value);
            }
            let response = request.send().await.map_err(|_| "transport failure")?;
            if !response.status().is_success() {
                return Err("R2 rejected an uploaded part");
            }
            let etag = response
                .headers()
                .get("etag")
                .and_then(|value| value.to_str().ok())
                .ok_or("R2 response did not expose ETag")?;
            uploaded.push(UploadedPart::new(number, etag).map_err(|_| "invalid returned ETag")?);
        }
        let manifest = CompletionManifest::try_from_parts(uploaded)
            .map_err(|_| "invalid completion manifest")?;
        session
            .complete(manifest)
            .await
            .map_err(|_| "complete failed")?;

        let object = client
            .as_sdk()
            .get_object()
            .bucket("r2kit-live-tests")
            .key(&key)
            .send()
            .await
            .map_err(|_| "get failed")?;
        let actual = object
            .body
            .collect()
            .await
            .map_err(|_| "body failed")?
            .into_bytes();
        let expected: Vec<u8> = chunks.into_iter().flatten().collect();
        if actual.as_ref() != expected {
            return Err("downloaded bytes differ");
        }
        Ok::<(), &'static str>(())
    }
    .await;

    let _ = session.abort().await;
    let _ = client
        .as_sdk()
        .delete_object()
        .bucket("r2kit-live-tests")
        .key(&key)
        .send()
        .await;
    result.unwrap();
}

#[tokio::test]
#[ignore = "requires explicit bucket-scoped R2 credentials"]
async fn live_presigned_multipart_abort_removes_upload() {
    let client = live_client();
    let bucket = client.bucket("r2kit-live-tests").unwrap();
    let key = format!("_r2kit-tests/{}/abort.bin", uuid::Uuid::new_v4());
    let session = bucket
        .presigned_multipart(&key)
        .unwrap()
        .file_size((6 * MIB) as u64)
        .part_size((5 * MIB) as u64)
        .create()
        .await
        .unwrap();
    let upload_id = session.snapshot().expose_upload_id().to_owned();

    session.abort().await.unwrap();

    let list_result = client
        .as_sdk()
        .list_parts()
        .bucket("r2kit-live-tests")
        .key(&key)
        .upload_id(upload_id)
        .send()
        .await;
    assert!(
        list_result.is_err(),
        "aborted multipart upload must no longer be listable"
    );

    let object_result = client
        .as_sdk()
        .head_object()
        .bucket("r2kit-live-tests")
        .key(&key)
        .send()
        .await;
    assert!(
        object_result.is_err(),
        "aborting must not create a completed object"
    );
}
