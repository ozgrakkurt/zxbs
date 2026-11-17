#![no_main]

use libfuzzer_sys::arbitrary::unstructured::Unstructured;
use libfuzzer_sys::fuzz_target;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use zxbs::cache::{self, CacheEntry};
use zxbs::hash::Hash;
use zxbs::thread_pool::ThreadPool;

fn thread_pool() -> &'static ThreadPool {
    static POOL: OnceLock<ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| ThreadPool::new(4))
}

fn temp_cache_dir() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("zxbs-fuzz-prune-{}-{}", std::process::id(), id));
    std::fs::remove_dir_all(&path).ok();
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn count(fuzz_input: &mut Unstructured, max: usize) -> usize {
    fuzz_input.choose_index(max).unwrap_or(0)
}

fuzz_target!(|data: &[u8]| {
    let mut fuzz_input = Unstructured::new(data);
    let cache_path = temp_cache_dir();

    let n_entries = count(&mut fuzz_input, 32);
    let mut entries = BTreeMap::new();
    let mut files: Vec<(Hash, PathBuf, u64)> = Vec::with_capacity(n_entries);

    for idx in 0..n_entries {
        let deep_hash = Hash::try_from(idx + 1).unwrap();
        let output_size = u32::try_from(count(&mut fuzz_input, 1024)).unwrap();
        let last_used_timestamp = fuzz_input.arbitrary::<u64>().unwrap_or(0);

        let path = cache_path.join(format!("{:x}", deep_hash));
        std::fs::write(&path, b"").unwrap();

        entries.insert(
            deep_hash,
            CacheEntry {
                last_used_timestamp,
                output_hash: 0,
                output_size,
            },
        );
        files.push((deep_hash, path, last_used_timestamp));
    }

    let total_size: u64 = entries
        .values()
        .map(|entry| u64::from(entry.output_size))
        .sum();
    let max_cache_size = u64::try_from(count(
        &mut fuzz_input,
        usize::try_from(total_size).unwrap_or(0) + 1,
    ))
    .unwrap();

    // Pruning only updates the state. Deleting the files of the evicted
    // entries happens afterwards, exactly like `cache::finish` does after it
    // writes the new state file.
    cache::prune_state(max_cache_size, &mut entries);

    std::fs::create_dir_all(cache_path.join("wal")).unwrap();
    cache::cleanup(&cache_path, thread_pool(), &entries);

    let remaining_size: u64 = entries
        .values()
        .map(|entry| u64::from(entry.output_size))
        .sum();
    assert!(remaining_size <= max_cache_size || entries.is_empty());

    for (deep_hash, path, _) in files.iter() {
        if entries.contains_key(deep_hash) {
            assert!(path.exists(), "a kept cache file was deleted");
        } else {
            assert!(!path.exists(), "an evicted cache file was not deleted");
        }
    }

    // Evicting the least recently used entries means nothing older than the
    // oldest remaining entry can have been kept.
    let oldest_remaining = entries
        .values()
        .map(|entry| entry.last_used_timestamp)
        .min()
        .unwrap_or(u64::MAX);
    for (deep_hash, _, last_used_timestamp) in files.iter() {
        if !entries.contains_key(deep_hash) {
            assert!(
                *last_used_timestamp <= oldest_remaining,
                "an evicted entry was newer than a kept entry"
            );
        }
    }

    std::fs::remove_dir_all(&cache_path).unwrap();
});
