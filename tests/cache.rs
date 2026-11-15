use std::path::Path;

use zxbs::cache;
use zxbs::hash::{Hash, hash_bytes};
use zxbs::thread_pool::ThreadPool;
use zxbs::timestamp::timestamp;

#[test]
fn test_cache_roundtrip() {
    let folder = format!("/tmp/opencode/zxbs-tests/{}", rand::random::<u128>());
    let cache_dir = format!("{}/cache", folder);
    let cache_path = Path::new(&cache_dir);

    let thread_pool = ThreadPool::new(2);

    let ts = timestamp();

    // A load on a fresh cache directory creates the directories and gives an
    // empty state.
    drop(cache::load(cache_path, &thread_pool, ts));

    // Store an output like the build system would: write the cached file and
    // a wal entry for it.
    let content: &[u8] = b"cached output contents";
    let deep: Hash = 0x1234567890abcdef1234567890abcdef;
    let output_hash = hash_bytes(content);
    std::fs::write(cache_path.join(format!("{:x}", deep)), content).unwrap();
    cache::write_wal_file(
        cache_path,
        deep,
        output_hash,
        u32::try_from(content.len()).unwrap(),
    );

    // A wal entry without a cached file next to it must be ignored.
    cache::write_wal_file(
        cache_path,
        deep + 1,
        output_hash,
        u32::try_from(content.len()).unwrap(),
    );

    // A wal entry whose cached file doesn't match the recorded output hash
    // must be ignored too.
    let wrong_content: &[u8] = b"totally different contents";
    std::fs::write(cache_path.join(format!("{:x}", deep + 2)), wrong_content).unwrap();
    cache::write_wal_file(
        cache_path,
        deep + 2,
        output_hash,
        u32::try_from(wrong_content.len()).unwrap(),
    );

    // Loading again picks up only the valid wal entry.
    let mut state = cache::load(cache_path, &thread_pool, ts);
    assert_eq!(state.item(deep, ts), Some(output_hash));

    // The invalid wal entries must not have made it in. This is checked on a
    // throwaway state because `item` records unknown hashes as new entries.
    let mut throwaway = cache::load(cache_path, &thread_pool, ts);
    assert_eq!(throwaway.item(deep + 1, ts), None);
    assert_eq!(throwaway.item(deep + 2, ts), None);
    drop(throwaway);

    // A brand new deep hash is unknown until the cached file is written and
    // the state is finished, just like the build system does during a run.
    let content2: &[u8] = b"another cached output";
    let deep2 = deep + 3;
    assert_eq!(state.item(deep2, ts), None);
    std::fs::write(cache_path.join(format!("{:x}", deep2)), content2).unwrap();
    cache::finish(state, cache_path, &thread_pool, ts);

    // Wal files are cleaned up after finish.
    let wal_count = std::fs::read_dir(cache_path.join("wal")).unwrap().count();
    assert_eq!(wal_count, 0);

    // A reload without any wal files reads the state file and still knows
    // both entries.
    let mut state = cache::load(cache_path, &thread_pool, ts);
    assert_eq!(state.item(deep, ts), Some(output_hash));
    assert_eq!(state.item(deep2, ts), Some(hash_bytes(content2)));
    cache::finish(state, cache_path, &thread_pool, ts);

    std::fs::remove_dir_all(&folder).unwrap();
}
