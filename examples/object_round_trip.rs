use r2kit::R2Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bucket_name = std::env::var("R2_BUCKET")?;
    let key = std::env::var("R2_KEY")?;
    let expected = b"hello from r2kit";

    let bucket = R2Client::from_env()?.bucket(bucket_name)?;
    bucket.put_bytes(&key, expected.to_vec()).await?;

    let downloaded = bucket.get(&key).await?;
    let actual = downloaded.into_body().collect().await?.into_bytes();
    assert_eq!(actual.as_ref(), expected);

    bucket.delete(key).await?;
    Ok(())
}
