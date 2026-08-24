use std::{
    env,
    sync::{Arc, Mutex},
};

use futures_util::{StreamExt, TryStreamExt, stream};
use r2kit::{R2Client, R2Config};

const MIB: usize = 1024 * 1024;

fn deep_live_client() -> Option<R2Client> {
    if env::var("R2KIT_DEEP_LIVE_TESTS").as_deref() != Ok("1") {
        return None;
    }
    assert_eq!(env::var("R2KIT_LIVE_TESTS").as_deref(), Ok("1"));
    assert_eq!(
        env::var("R2KIT_LIVE_BUCKET").as_deref(),
        Ok("r2kit-live-tests")
    );
    Some(R2Client::new(
        R2Config::from_env().expect("R2 live credentials are required"),
    ))
}

fn deterministic_bytes(length: usize) -> Vec<u8> {
    let mut state = 0x4d59_5df4_d0f3_3173_u64;
    let mut bytes = Vec::with_capacity(length);
    for _ in 0..length {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push(state as u8);
    }
    bytes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires explicit deep live testing against dedicated R2 credentials"]
async fn live_parallel_64_mib_upload_is_exact_and_progress_is_monotonic() {
    let Some(client) = deep_live_client() else {
        return;
    };
    let bucket = client.bucket("r2kit-live-tests").unwrap();
    let key = format!("_r2kit-tests/{}/stress.bin", uuid::Uuid::new_v4());
    let expected = deterministic_bytes(64 * MIB);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stress.bin");
    std::fs::write(&path, &expected).unwrap();
    let progress = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&progress);

    let result = async {
        let upload = bucket
            .managed_multipart(&key)
            .map_err(|_| "managed builder failed")?
            .part_size((8 * MIB) as u64)
            .concurrency(8)
            .max_attempts(4)
            .on_progress(move |event| {
                observed.lock().unwrap().push((
                    event.completed_parts(),
                    event.transferred_bytes(),
                    event.total_parts(),
                    event.total_bytes(),
                ));
            })
            .upload_file(&path)
            .await
            .map_err(|_| "managed stress upload failed")?;

        if upload.file_size() != expected.len() as u64
            || upload.part_count() != 8
            || upload.uploaded_parts() != 8
            || upload.reused_parts() != 0
        {
            return Err("managed upload result counters differ");
        }

        let events = progress.lock().unwrap().clone();
        if events.len() != 9 || events.first() != Some(&(0, 0, 8, (64 * MIB) as u64)) {
            return Err("progress did not report the initial and eight completed states");
        }
        if events.windows(2).any(|pair| {
            pair[0].0 >= pair[1].0
                || pair[0].1 >= pair[1].1
                || pair[1].2 != 8
                || pair[1].3 != (64 * MIB) as u64
        }) {
            return Err("parallel progress callbacks were not strictly monotonic");
        }

        let object = bucket.get(&key).await.map_err(|_| "stress GET failed")?;
        if object.metadata().size() != expected.len() as u64 {
            return Err("stress object metadata size differs");
        }
        let actual = object
            .into_body()
            .collect()
            .await
            .map_err(|_| "stress body collection failed")?
            .into_bytes();
        if actual.as_ref() != expected.as_slice() {
            return Err("stress object bytes differ");
        }
        Ok::<(), &'static str>(())
    }
    .await;

    let _ = bucket.delete(&key).await;
    result.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "creates 1,001 objects and requires explicit deep live R2 testing"]
async fn live_batch_delete_chunks_more_than_one_thousand_keys() {
    let Some(client) = deep_live_client() else {
        return;
    };
    let bucket = client.bucket("r2kit-live-tests").unwrap();
    let prefix = format!("_r2kit-tests/{}/batch-delete/", uuid::Uuid::new_v4());
    let keys = (0..1_001)
        .map(|index| format!("{prefix}{index:04}.bin"))
        .collect::<Vec<_>>();

    let result = async {
        let uploaded = stream::iter(keys.iter().map(|key| {
            let bucket = bucket.clone();
            async move {
                bucket
                    .put_bytes(key, Vec::new())
                    .await
                    .map(|_| key)
                    .map_err(|_| "stress object PUT failed")
            }
        }))
        .buffer_unordered(32)
        .try_collect::<Vec<_>>()
        .await?;
        if uploaded.len() != keys.len() {
            return Err("not every stress object was uploaded");
        }

        let deleted = bucket
            .delete_objects(keys.clone())
            .await
            .map_err(|_| "multi-request batch delete failed")?;
        if deleted.request_count() != 2
            || deleted.deleted_keys().len() != keys.len()
            || !deleted.failures().is_empty()
        {
            return Err("batch delete did not complete as two successful requests");
        }

        let page = bucket
            .list()
            .prefix(&prefix)
            .limit(1)
            .send()
            .await
            .map_err(|_| "post-delete listing failed")?;
        if !page.objects().is_empty() || page.next_continuation_token().is_some() {
            return Err("batch-deleted objects remain visible");
        }
        Ok::<(), &'static str>(())
    }
    .await;

    let _ = bucket.delete_objects(keys).await;
    result.unwrap();
}
