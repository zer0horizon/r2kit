use std::env;

use r2kit::{Error, R2Client, R2Config};

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
async fn live_core_object_round_trip_and_pagination() {
    let client = live_client();
    let bucket = client.bucket("r2kit-live-tests").unwrap();
    let prefix = format!("_r2kit-tests/{}/objects/", uuid::Uuid::new_v4());
    let first_key = format!("{prefix}a.txt");
    let second_key = format!("{prefix}b.txt");
    let first_body = b"r2kit object API: first".to_vec();
    let second_body = b"r2kit object API: second".to_vec();

    let result = async {
        let put = bucket
            .put_bytes(&first_key, first_body.clone())
            .await
            .map_err(|_| "first put failed")?;
        bucket
            .put_bytes(&second_key, second_body)
            .await
            .map_err(|_| "second put failed")?;

        let metadata = bucket.head(&first_key).await.map_err(|_| "head failed")?;
        if metadata.size() != first_body.len() as u64 || metadata.etag() != put.etag() {
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

        let first_page = bucket
            .list()
            .prefix(&prefix)
            .limit(1)
            .send()
            .await
            .map_err(|_| "first list page failed")?;
        if first_page.objects().len() != 1 {
            return Err("first page must contain one object");
        }
        let token = first_page
            .next_continuation_token()
            .ok_or("first page must expose a continuation token")?;
        let second_page = bucket
            .list()
            .prefix(&prefix)
            .limit(1)
            .continuation_token(token)
            .send()
            .await
            .map_err(|_| "second list page failed")?;
        if second_page.objects().len() != 1 {
            return Err("second page must contain one object");
        }
        Ok::<(), &'static str>(())
    }
    .await;

    let _ = bucket.delete(&first_key).await;
    let _ = bucket.delete(&second_key).await;
    result.unwrap();

    bucket.delete(&first_key).await.unwrap();
    assert!(matches!(
        bucket.head(&first_key).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(bucket.get(&first_key).await, Err(Error::NotFound)));
}
