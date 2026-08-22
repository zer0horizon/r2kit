use r2kit::MultipartSessionSnapshot;

#[test]
fn multipart_snapshot_exposes_every_persistence_field_deliberately() {
    let snapshot = MultipartSessionSnapshot::restore(
        "example-bucket",
        "videos/example.mp4",
        "sensitive-upload-id",
        11 * 1024 * 1024,
        5 * 1024 * 1024,
    )
    .unwrap();

    assert_eq!(snapshot.bucket(), "example-bucket");
    assert_eq!(snapshot.key(), "videos/example.mp4");
    assert_eq!(snapshot.expose_upload_id(), "sensitive-upload-id");
    assert_eq!(snapshot.file_size(), 11 * 1024 * 1024);
    assert_eq!(snapshot.part_size(), 5 * 1024 * 1024);

    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("sensitive-upload-id"));
}
