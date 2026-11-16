#![no_main]

use libfuzzer_sys::arbitrary::unstructured::Unstructured;
use libfuzzer_sys::fuzz_target;

use std::collections::BTreeMap;
use zxbs::cache::{
    CacheEntry, WalEntry, deserialize_wal_file, serialize_state_file, serialize_wal_file,
    try_deserialize_state_file,
};
use zxbs::hash::Hash;

fn arbitrary_hash(fuzz_input: &mut Unstructured) -> Option<Hash> {
    fuzz_input.arbitrary::<Hash>().ok()
}

fuzz_target!(|data: &[u8]| {
    // Arbitrary bytes must be rejected without panicking. This exercises the
    // unsafe unaligned reads in the parsers.
    let _ = deserialize_wal_file(data);
    let _ = try_deserialize_state_file(data);

    let mut fuzz_input = Unstructured::new(data);

    // WAL roundtrip.
    for _ in 0..4 {
        let Some(deep_hash) = arbitrary_hash(&mut fuzz_input) else {
            return;
        };
        let Some(output_hash) = arbitrary_hash(&mut fuzz_input) else {
            return;
        };
        let Ok(output_size) = fuzz_input.arbitrary::<u32>() else {
            return;
        };

        let entry = WalEntry {
            deep_hash,
            output_hash,
            output_size,
        };

        let mut out = Vec::new();
        serialize_wal_file(&entry, &mut out);
        assert_eq!(deserialize_wal_file(&out), Ok(entry));
    }

    // State file roundtrip.
    let n_entries = fuzz_input.choose_index(16).ok().unwrap_or(0);
    let mut entries = BTreeMap::new();
    for _ in 0..n_entries {
        let Some(deep_hash) = arbitrary_hash(&mut fuzz_input) else {
            return;
        };
        let Some(output_hash) = arbitrary_hash(&mut fuzz_input) else {
            return;
        };
        let Ok(output_size) = fuzz_input.arbitrary::<u32>() else {
            return;
        };
        let Ok(last_used_timestamp) = fuzz_input.arbitrary::<u64>() else {
            return;
        };
        entries.insert(
            deep_hash,
            CacheEntry {
                last_used_timestamp,
                output_hash,
                output_size,
            },
        );
    }

    let mut out = Vec::new();
    serialize_state_file(&entries, &mut out);
    assert_eq!(try_deserialize_state_file(&out), Ok(entries));

    // Flipping any single byte must be detected. The checksum covers both the
    // header and the entries, so every bit flip is a corruption.
    if !out.is_empty() {
        let idx = fuzz_input.choose_index(out.len()).unwrap();
        out[idx] ^= 1;
        assert!(try_deserialize_state_file(&out).is_err());
    }
});
