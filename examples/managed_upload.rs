use std::{io, path::PathBuf};

use r2kit::R2Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file path"))?;
    let bucket_name = std::env::var("R2_BUCKET")?;
    let key = std::env::var("R2_KEY")?;

    let bucket = R2Client::from_env()?.bucket(bucket_name)?;
    let result = bucket
        .managed_multipart(key)?
        .concurrency(4)
        .max_attempts(4)
        .on_progress(|progress| {
            eprintln!(
                "uploaded {}/{} bytes",
                progress.transferred_bytes(),
                progress.total_bytes()
            );
        })
        .upload_file(path)
        .await?;

    eprintln!("completed {} multipart parts", result.part_count());
    Ok(())
}
