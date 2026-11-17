use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zxbs::cache::{self, CacheEntry};
use zxbs::hash::{Hash, hash_bytes};
use zxbs::thread_pool::ThreadPool;
use zxbs::timestamp::timestamp;

mod tmp_dir;
use tmp_dir::TempDir;

fn make_cache_dir() -> TempDir {
    TempDir::new()
}

#[test]
fn test_cache_roundtrip() {
    let folder = TempDir::new();
    let cache_dir = format!("{}/cache", folder.as_str());
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

    // The invalid recovery candidate's cached file is not referenced by the
    // final state, so cleanup removes it too.
    assert!(!cache_path.join(format!("{:x}", deep + 2)).exists());
    assert!(cache_path.join("cache_state").exists());

    // A reload without any wal files reads the state file and still knows
    // both entries.
    let mut state = cache::load(cache_path, &thread_pool, ts);
    assert_eq!(state.item(deep, ts), Some(output_hash));
    assert_eq!(state.item(deep2, ts), Some(hash_bytes(content2)));
    cache::finish(state, cache_path, &thread_pool, ts);
}

// A wal entry that is structurally valid and points at a valid cached file can
// still be rejected. Recovery must not trust a wal file blindly because it may
// have been partially written or tampered with.
#[test]
fn test_invalid_wal_recovery_is_ignored() {
    let folder = TempDir::new();
    let cache_dir = format!("{}/cache", folder.as_str());
    let cache_path = Path::new(&cache_dir);

    let thread_pool = ThreadPool::new(2);
    let ts = timestamp();

    drop(cache::load(cache_path, &thread_pool, ts));

    let content: &[u8] = b"cached output";
    let output_hash = hash_bytes(content);
    let output_size = u32::try_from(content.len()).unwrap();

    let write_cached_file = |deep: Hash| {
        std::fs::write(cache_path.join(format!("{:x}", deep)), content).unwrap();
    };

    // The recorded output size doesn't match the cached file.
    let wrong_size: Hash = 1;
    write_cached_file(wrong_size);
    cache::write_wal_file(cache_path, wrong_size, output_hash, output_size + 1);

    // The wal file is named after one deep hash but records another. This can
    // happen if a wal file gets renamed or partially rewritten.
    let embedded_deep: Hash = 2;
    let wrong_name: Hash = 3;
    write_cached_file(wrong_name);
    cache::write_wal_file(cache_path, embedded_deep, output_hash, output_size);
    std::fs::rename(
        cache_path.join("wal").join(format!("{:x}", embedded_deep)),
        cache_path.join("wal").join(format!("{:x}", wrong_name)),
    )
    .unwrap();

    // The checksum of the wal file was corrupted.
    let bad_checksum: Hash = 4;
    write_cached_file(bad_checksum);
    cache::write_wal_file(cache_path, bad_checksum, output_hash, output_size);
    let wal_path = cache_path.join("wal").join(format!("{:x}", bad_checksum));
    let mut bytes = std::fs::read(&wal_path).unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    std::fs::write(&wal_path, bytes).unwrap();

    // The wal file was truncated in the middle of an entry.
    let truncated: Hash = 5;
    write_cached_file(truncated);
    cache::write_wal_file(cache_path, truncated, output_hash, output_size);
    let wal_path = cache_path.join("wal").join(format!("{:x}", truncated));
    let bytes = std::fs::read(&wal_path).unwrap();
    std::fs::write(&wal_path, &bytes[..bytes.len() / 2]).unwrap();

    // A valid entry to make sure the invalid ones were not ignored just
    // because recovery is broken as a whole.
    let valid: Hash = 6;
    write_cached_file(valid);
    cache::write_wal_file(cache_path, valid, output_hash, output_size);

    let mut state = cache::load(cache_path, &thread_pool, ts);
    assert_eq!(state.item(valid, ts), Some(output_hash));
    for deep in [wrong_size, wrong_name, bad_checksum, truncated] {
        assert_eq!(state.item(deep, ts), None);
    }
}

// A cache directory can exist without a wal directory, for example after a
// crash or after an older version of the build system created it.
#[test]
fn test_load_creates_missing_wal_directory() {
    let cache_path = make_cache_dir();
    assert!(!cache_path.join("wal").exists());

    let thread_pool = ThreadPool::new(1);
    let ts = timestamp();

    let mut state = cache::load(&cache_path, &thread_pool, ts);

    assert!(cache_path.join("wal").is_dir());
    assert_eq!(state.item(42, ts), None);
}

fn cache_entry(last_used_timestamp: u64, output_size: u32) -> CacheEntry {
    CacheEntry {
        last_used_timestamp,
        output_hash: 0,
        output_size,
    }
}

// Creates the cached file for an entry and returns its path.
fn write_cached_file(cache_path: &Path, deep_hash: Hash, size: usize) -> PathBuf {
    let path = cache_path.join(format!("{:x}", deep_hash));
    std::fs::write(&path, vec![0u8; size]).unwrap();
    path
}

// Entries are given as (deep_hash, last_used_timestamp) and all entries get
// the same output size so the tests can reason about the total cache size.
// Returns the entries map and the path of each entry's cached file, in the
// same order as the input list.
fn setup_entries(
    cache_path: &Path,
    entry_size: u32,
    entries: &[(Hash, u64)],
) -> (BTreeMap<Hash, CacheEntry>, Vec<PathBuf>) {
    let mut map = BTreeMap::new();
    let mut files = Vec::with_capacity(entries.len());

    for &(deep_hash, last_used_timestamp) in entries.iter() {
        map.insert(deep_hash, cache_entry(last_used_timestamp, entry_size));
        files.push(write_cached_file(
            cache_path,
            deep_hash,
            usize::try_from(entry_size).unwrap(),
        ));
    }

    (map, files)
}

// `prune_state` only updates the state map. Deleting evicted files is done by
// `cleanup` in `finish` after the final state has been written.
fn prune_state(entries: &mut BTreeMap<Hash, CacheEntry>, max_cache_size: u64) {
    cache::prune_state(max_cache_size, entries);
}

#[test]
fn prune_state_keeps_everything_when_at_or_under_the_limit() {
    let cache_path = make_cache_dir();

    // 3 entries * 10 bytes = 30 bytes total, exactly at the limit.
    let (mut entries, files) = setup_entries(&cache_path, 10, &[(1, 100), (2, 200), (3, 300)]);

    prune_state(&mut entries, 30);

    assert_eq!(entries.len(), 3);
    for file in files.iter() {
        assert!(
            file.exists(),
            "{} should not have been deleted",
            file.display()
        );
    }
}

#[test]
fn prune_state_evicts_the_oldest_entries_first() {
    let cache_path = make_cache_dir();

    // 4 entries * 100 bytes = 400 bytes total. The newest entry is first in
    // the list to make sure the code does not just trust insertion order.
    let (mut entries, files) =
        setup_entries(&cache_path, 100, &[(4, 400), (2, 200), (1, 100), (3, 300)]);

    // Evicting the two oldest (200 bytes) gets us down to 200 <= 250.
    prune_state(&mut entries, 250);

    assert_eq!(entries.len(), 2);
    let remaining_timestamps: Vec<u64> = entries
        .values()
        .map(|entry| entry.last_used_timestamp)
        .collect();
    assert_eq!(remaining_timestamps, vec![300, 400]);

    // prune_state does not touch the filesystem; finish/cleanup removes the
    // evicted files after writing the new state.
    for file in files.iter() {
        assert!(
            file.exists(),
            "prune_state must not delete files itself: {}",
            file.display()
        );
    }
}

#[test]
fn prune_state_stops_as_soon_as_it_is_under_the_limit() {
    let cache_path = make_cache_dir();

    // 4 entries * 100 bytes = 400 bytes total.
    let (mut entries, files) =
        setup_entries(&cache_path, 100, &[(1, 100), (2, 200), (3, 300), (4, 400)]);

    // Evicting only the oldest entry gets us to 300, which is not over the
    // limit, so exactly one entry must be evicted.
    prune_state(&mut entries, 300);

    assert_eq!(entries.len(), 3);
    assert!(!entries.contains_key(&1), "oldest entry was not evicted");

    // Again, file deletion is cleanup's job.
    for file in files.iter() {
        assert!(file.exists(), "unexpected deletion of {}", file.display());
    }
}

#[test]
fn test_finish_cleans_unreferenced_files() {
    let folder = TempDir::new();
    let cache_path = folder.join("cache");
    let thread_pool = ThreadPool::new(4);
    let ts = timestamp();

    drop(cache::load(&cache_path, &thread_pool, ts));

    // An existing entry that must survive finish.
    let keep: Hash = 0x10;
    let keep_content: &[u8] = b"keep";
    let keep_output_hash = hash_bytes(keep_content);
    let mut entries = BTreeMap::new();
    entries.insert(
        keep,
        CacheEntry {
            last_used_timestamp: ts,
            output_hash: keep_output_hash,
            output_size: u32::try_from(keep_content.len()).unwrap(),
        },
    );

    let mut state_bytes = Vec::new();
    cache::serialize_state_file(&entries, &mut state_bytes);
    std::fs::write(cache_path.join("cache_state"), &state_bytes).unwrap();
    std::fs::write(cache_path.join(format!("{:x}", keep)), keep_content).unwrap();

    let mut state = cache::load(&cache_path, &thread_pool, ts);
    assert_eq!(state.item(keep, ts), Some(keep_output_hash));

    // A new entry whose cached file is written before finish.
    let new: Hash = 0x11;
    let new_content: &[u8] = b"new";
    assert_eq!(state.item(new, ts), None);
    std::fs::write(cache_path.join(format!("{:x}", new)), new_content).unwrap();

    // Files that cleanup must remove: an orphan cache file, a stale temp file,
    // a stale state temp, and a leftover wal file. Creating cache_state.tmp
    // also checks that it does not race with write_state_file.
    let orphan: Hash = 0x12;
    std::fs::write(cache_path.join(format!("{:x}", orphan)), b"orphan").unwrap();
    let stale_tmp = cache_path.join("stale.tmp");
    std::fs::write(&stale_tmp, b"tmp").unwrap();
    let state_tmp = cache_path.join("cache_state.tmp");
    std::fs::write(&state_tmp, b"state tmp").unwrap();
    let wal_junk = cache_path.join("wal").join("junk");
    std::fs::write(&wal_junk, b"wal").unwrap();

    cache::finish(state, &cache_path, &thread_pool, ts);

    // Referenced cache files and the state file survive.
    assert!(cache_path.join(format!("{:x}", keep)).exists());
    assert!(cache_path.join(format!("{:x}", new)).exists());
    assert!(cache_path.join("cache_state").exists());

    // Unreferenced files, temps and wal files are gone.
    assert!(!cache_path.join(format!("{:x}", orphan)).exists());
    assert!(!stale_tmp.exists());
    assert!(!state_tmp.exists());
    assert!(!wal_junk.exists());

    let mut loaded = cache::load(&cache_path, &thread_pool, ts);
    assert_eq!(loaded.item(keep, ts), Some(keep_output_hash));
    assert_eq!(loaded.item(new, ts), Some(hash_bytes(new_content)));
}

#[test]
fn test_finish_prunes_and_removes_evicted_files() {
    let folder = TempDir::new();
    let cache_path = folder.join("cache");
    let thread_pool = ThreadPool::new(2);
    let ts = timestamp();

    drop(cache::load(&cache_path, &thread_pool, ts));

    let keep: Hash = 0x20;
    let keep_content: &[u8] = b"keep";
    let keep_output_hash = hash_bytes(keep_content);

    // Use an exaggerated output_size to trigger pruning without creating a
    // gigabyte-sized cache file.
    let evict: Hash = 0x21;
    let evict_content: &[u8] = b"evict";
    let evict_output_hash = hash_bytes(evict_content);

    let mut entries = BTreeMap::new();
    entries.insert(
        evict,
        CacheEntry {
            last_used_timestamp: 100,
            output_hash: evict_output_hash,
            output_size: u32::MAX,
        },
    );
    entries.insert(
        keep,
        CacheEntry {
            last_used_timestamp: 200,
            output_hash: keep_output_hash,
            output_size: u32::try_from(keep_content.len()).unwrap(),
        },
    );

    let mut state_bytes = Vec::new();
    cache::serialize_state_file(&entries, &mut state_bytes);
    std::fs::write(cache_path.join("cache_state"), &state_bytes).unwrap();
    std::fs::write(cache_path.join(format!("{:x}", evict)), evict_content).unwrap();
    std::fs::write(cache_path.join(format!("{:x}", keep)), keep_content).unwrap();

    // A new entry is merged after pruning, so it must not be pruned even though
    // the old state is over the size limit.
    let mut state = cache::load(&cache_path, &thread_pool, ts);
    let new: Hash = 0x22;
    let new_content: &[u8] = b"new after prune";
    assert_eq!(state.item(new, ts), None);
    std::fs::write(cache_path.join(format!("{:x}", new)), new_content).unwrap();

    cache::finish(state, &cache_path, &thread_pool, ts);

    // The older oversized entry is evicted from state, and cleanup removes its
    // file. The newer entries and their files survive.
    assert!(!cache_path.join(format!("{:x}", evict)).exists());
    assert!(cache_path.join(format!("{:x}", keep)).exists());
    assert!(cache_path.join(format!("{:x}", new)).exists());

    let mut loaded = cache::load(&cache_path, &thread_pool, ts);
    assert_eq!(loaded.item(keep, ts), Some(keep_output_hash));
    assert_eq!(loaded.item(new, ts), Some(hash_bytes(new_content)));
    assert_eq!(loaded.item(evict, ts), None);
}
