use std::{io, path::PathBuf, time::Duration};

use r2kit::{CacheControl, R2Client};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file path"))?;
    let bucket_name = std::env::var("R2_BUCKET")?;
    let key = std::env::var("R2_KEY")?;
    let content_type = mime_guess::from_path(&path).first_or_octet_stream();
    let cache_control = CacheControl::new()
        .with_public()
        .with_max_age(Duration::from_secs(3_600));

    let bucket = R2Client::from_env()?.bucket(bucket_name)?;
    let result = bucket
        .managed_multipart(key)?
        .content_type(content_type)
        .cache_control(cache_control)
        .concurrency(4)
        .max_attempts(4)
        .on_progress(|progress| {
            eprintln!(
                "uploaded {}/{} bytes ({:.1}%)",
                progress.transferred_bytes(),
                progress.total_bytes(),
                progress.percentage(),
            );
        })
        .upload_file(path)
        .await?;

    eprintln!("completed {} multipart parts", result.part_count());
    Ok(())
}
