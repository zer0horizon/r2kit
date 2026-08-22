#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use r2kit::{
    CompletionManifest, MultipartPartReceipt, MultipartSessionRecord, MultipartSessionSnapshot,
    PartMd5, PartNumber,
};

#[derive(Arbitrary, Debug)]
struct ProtocolInput {
    bucket: String,
    key: String,
    upload_id: String,
    file_size: u64,
    part_size: u64,
    md5: String,
    receipts: Vec<(u16, String)>,
}

fuzz_target!(|input: ProtocolInput| {
    let _ = PartNumber::try_from(input.receipts.first().map_or(0, |part| part.0));
    let _ = PartMd5::try_from(input.md5);

    let receipts = input
        .receipts
        .into_iter()
        .take(10_001)
        .map(|(number, etag)| MultipartPartReceipt::new(number, etag));
    let _ = CompletionManifest::try_from_receipts(receipts);

    if let Ok(snapshot) = MultipartSessionSnapshot::restore(
        input.bucket,
        input.key,
        input.upload_id,
        input.file_size,
        input.part_size,
    ) {
        let bucket = snapshot.bucket().to_owned();
        let key = snapshot.key().to_owned();
        let file_size = snapshot.file_size();
        let part_size = snapshot.part_size();
        let record = snapshot.into_persistence_record();
        let encoded = serde_json::to_vec(&record).expect("valid record must serialize");
        let decoded: MultipartSessionRecord =
            serde_json::from_slice(&encoded).expect("serialized record must deserialize");
        let restored = MultipartSessionSnapshot::from_persistence_record(decoded)
            .expect("library-produced record must restore");
        assert_eq!(restored.bucket(), bucket);
        assert_eq!(restored.key(), key);
        assert_eq!(restored.file_size(), file_size);
        assert_eq!(restored.part_size(), part_size);
    }
});
