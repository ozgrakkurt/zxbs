use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind as IoErrorKind;
use std::mem::size_of;
use std::path::Path;

use crate::hash::{Hash, hash_bytes};
use crate::thread_pool::{JoinSet, ThreadPool};

fn read_bytes<T>(content: &[u8], offset: &mut usize) -> Result<T, ()> {
    let offs = *offset;
    let end = size_of::<T>() + offs;

    if end > content.len() {
        return Err(());
    }

    *offset = end;

    // Bite me.
    Ok(unsafe { std::ptr::read_unaligned(content.as_ptr().add(offs) as *const T) })
}

struct CacheEntry {
    last_used_timestamp: u64,
    output_hash: Hash,
    output_size: u32,
}

struct WalEntry {
    deep_hash: Hash,
    output_hash: Hash,
    output_size: u32,
}

pub struct CacheState {
    // map deep hash to Entry
    entries: BTreeMap<Hash, CacheEntry>,
    // list of new cache entries' deep hashes inserted in this run of the build system.
    // we keep these seperately because they won't have output_hash yet.
    // we will calculate output_hashes of these after the build is done in a separate step.
    new_entries: BTreeSet<Hash>,
}

impl CacheState {
    fn empty() -> CacheState {
        CacheState {
            entries: BTreeMap::default(),
            new_entries: BTreeSet::new(),
        }
    }

    // Docs: Just read the code below.
    pub fn item(&mut self, deep_hash: Hash, timestamp: u64) -> Option<Hash> {
        match self.entries.get_mut(&deep_hash) {
            Some(v) => {
                v.last_used_timestamp = timestamp;
                Some(v.output_hash)
            }
            None => {
                assert!(self.new_entries.insert(deep_hash));
                None
            }
        }
    }
}

const WAL_FILE_MAGIC: u64 = 0x104aedd95bce65d6u64;
const STATE_FILE_MAGIC: u64 = 0x167045aafea85f3du64;

// CACHED FILE FORMAT
// name: <deep_hash>
// content: <output content>
//
// The checksum is hash_bytes(<all file content that came before the checksum>);
//
// location: <cache_path>/
//
// We don't ever expect cached files to be corrupted.
// We will hard error if a cached file is broken.

// We don't provide write/read serialize/deserialize functions for cached files here because
// it is just a std::write or std::read and the part of the program that handles that file
// is responsible for the integrity of a cached file.

// STATE FILE FORMAT
// name: "cache_state"
// content: <STATE_FILE_MAGIC> <number_of_entries:u32> <entry>* <checksum>
// location: <cache_path>/
//
// The checksum is hash_bytes(<all file content that came before the checksum>);
//
// Each entry is <deep_hash> <last_used_timestamp> <output_hash> <output_size>
//
// We don't expect the state file to ever be corrupted.
// Finding a problem in the loaded state file means we will hard error.
//
// State file is read at program start and written at program end.

fn serialize_state_file(entries: &BTreeMap<Hash, CacheEntry>, out: &mut Vec<u8>) {
    let start_offset = out.len();

    out.extend_from_slice(&STATE_FILE_MAGIC.to_le_bytes());

    let num_entries = u32::try_from(entries.len()).unwrap();

    out.extend_from_slice(&num_entries.to_le_bytes());

    for (deep_hash, entry) in entries.iter() {
        out.extend_from_slice(&deep_hash.to_le_bytes());
        out.extend_from_slice(&entry.last_used_timestamp.to_le_bytes());
        out.extend_from_slice(&entry.output_hash.to_le_bytes());
        out.extend_from_slice(&entry.output_size.to_le_bytes());
    }

    let checksum = hash_bytes(&out.as_slice()[start_offset..]);
    out.extend_from_slice(&checksum.to_le_bytes());
}

fn deserialize_state_file(content: &[u8]) -> BTreeMap<Hash, CacheEntry> {
    let mut offset: usize = 0;

    let magic = read_bytes::<u64>(content, &mut offset).unwrap();

    assert!(magic == STATE_FILE_MAGIC);

    let num_entries = read_bytes::<u32>(content, &mut offset).unwrap();

    let mut out: BTreeMap<Hash, CacheEntry> = BTreeMap::new();
    for _ in 0..num_entries {
        let deep_hash = read_bytes::<Hash>(content, &mut offset).unwrap();
        let last_used_timestamp = read_bytes::<u64>(content, &mut offset).unwrap();
        let output_hash = read_bytes::<Hash>(content, &mut offset).unwrap();
        let output_size = read_bytes::<u32>(content, &mut offset).unwrap();

        assert!(
            out.insert(
                deep_hash,
                CacheEntry {
                    last_used_timestamp,
                    output_hash,
                    output_size,
                }
            )
            .is_none()
        );
    }

    let checksum_check = hash_bytes(&content[..offset]);

    let checksum = read_bytes::<Hash>(content, &mut offset).unwrap();

    assert!(checksum_check == checksum);

    assert!(offset == content.len());

    out
}

// WAL FILE FORMAT
// name: <deep_hash>
// location: <cache_path>/wal/
// content: <WAL_FILE_MAGIC> <deep_hash> <output_hash> <output_size> <checksum>
//
// The checksum is hash_bytes(<all file content that came before the checksum>);
//
// There is supposed to be a file in <cache_path>/<deep_hash> that corresponds
// to the wal entry.
//
// We will check the cached file is there and it is valid and consistent with the wal file
// when processing the wal files for recovery. Inconsistent wal files are just ignored.
//
// We will delete all wal files after we finish recovery step in cache loading.

fn serialize_wal_file(entry: &WalEntry, out: &mut Vec<u8>) {
    let start_offset = out.len();

    out.extend_from_slice(&WAL_FILE_MAGIC.to_le_bytes());

    out.extend_from_slice(&entry.deep_hash.to_le_bytes());
    out.extend_from_slice(&entry.output_hash.to_le_bytes());
    out.extend_from_slice(&entry.output_size.to_le_bytes());

    let checksum = hash_bytes(&out.as_slice()[start_offset..]);
    out.extend_from_slice(&checksum.to_le_bytes());
}

pub fn write_wal_file(cache_path: &Path, deep_hash: Hash, output_hash: Hash, output_size: u32) {
    let mut out = Vec::new();
    serialize_wal_file(
        &WalEntry {
            deep_hash,
            output_hash,
            output_size,
        },
        &mut out,
    );

    let out_path = cache_path.join("wal").join(format!("{:x}", deep_hash));

    std::fs::write(&out_path, out.as_slice()).unwrap();
}

fn deserialize_wal_file(content: &[u8]) -> Result<WalEntry, ()> {
    let mut offset: usize = 0;

    let magic = read_bytes::<u64>(content, &mut offset)?;

    if magic != WAL_FILE_MAGIC {
        return Err(());
    }

    let deep_hash = read_bytes::<Hash>(content, &mut offset)?;
    let output_hash = read_bytes::<Hash>(content, &mut offset)?;
    let output_size = read_bytes::<u32>(content, &mut offset)?;

    let checksum_check = hash_bytes(&content[..offset]);

    let checksum = read_bytes::<Hash>(content, &mut offset)?;

    if content.len() != offset {
        return Err(());
    }

    if checksum_check != checksum {
        return Err(());
    }

    Ok(WalEntry {
        deep_hash,
        output_hash,
        output_size,
    })
}

fn load_wal(
    cache_path: &Path,
    wal_path: &Path,
    thread_pool: &ThreadPool,
    timestamp: u64,
    entries: &mut BTreeMap<Hash, CacheEntry>,
) {
    let dir = match std::fs::read_dir(wal_path) {
        Ok(d) => d,
        Err(e) => {
            assert!(e.kind() == IoErrorKind::NotFound);
            return;
        }
    };

    let mut join_set: JoinSet<Option<(Hash, CacheEntry)>> = JoinSet::new();

    for entry in dir {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file());

        let file_name = entry.file_name();
        let name = file_name.to_str().unwrap();
        let deep_hash = Hash::from_str_radix(name, 16).unwrap();

        let wal_file_path = entry.path();
        let cached_file_path = cache_path.join(name);

        join_set.spawn(thread_pool, move || {
            let contents = std::fs::read(&wal_file_path).unwrap();

            let wal_entry = match deserialize_wal_file(&contents) {
                Ok(v) => v,
                Err(_) => return None,
            };

            if wal_entry.deep_hash != deep_hash {
                return None;
            }

            let cached_file_content = match std::fs::read(&cached_file_path) {
                Ok(v) => v,
                Err(e) => {
                    assert!(e.kind() == IoErrorKind::NotFound);
                    return None;
                }
            };

            if hash_bytes(&cached_file_content) != wal_entry.output_hash {
                return None;
            }
            if cached_file_content.len() != usize::try_from(wal_entry.output_size).unwrap() {
                return None;
            }

            Some((
                deep_hash,
                CacheEntry {
                    last_used_timestamp: timestamp,
                    output_size: wal_entry.output_size,
                    output_hash: wal_entry.output_hash,
                },
            ))
        });
    }

    while let Some(res) = join_set.join_next() {
        if let Some((deep_hash, cache_entry)) = res {
            entries.insert(deep_hash, cache_entry);
        }
    }
}

// - Create the <cache-path> and <cache-path>/wal directories if they don't exist.
// - Load the state file.
// - Load and validate wal files.
// - Merge state_file with state coming from wal files.
pub fn load(cache_path: &Path, thread_pool: &ThreadPool, timestamp: u64) -> CacheState {
    let wal_path = cache_path.join("wal");

    if !std::fs::exists(cache_path).unwrap() {
        std::fs::create_dir_all(cache_path).unwrap();
        std::fs::create_dir(&wal_path).unwrap();

        return CacheState::empty();
    }

    if !std::fs::exists(&wal_path).unwrap() {
        std::fs::create_dir(&wal_path).unwrap();
    }

    let state_file_path = cache_path.join("cache_state");

    let mut entries: BTreeMap<Hash, CacheEntry> = match std::fs::read(&state_file_path) {
        Ok(v) => deserialize_state_file(&v),
        Err(e) => {
            assert!(e.kind() == IoErrorKind::NotFound);
            BTreeMap::new()
        }
    };

    load_wal(cache_path, &wal_path, thread_pool, timestamp, &mut entries);

    CacheState {
        entries,
        new_entries: BTreeSet::new(),
    }
}

// New entries only record their deep hashes because their outputs are not
// produced yet when they are inserted during the build. The outputs are
// stored as cached files in <cache_path>/<deep_hash> by the build system, so
// we read them back here, calculate their output hashes and sizes and merge
// them into the existing entries.
//
// A new entry without a cached file means there is something seriously wrong so we hard error in that case.
fn merge_new_entries(
    cache_path: &Path,
    thread_pool: &ThreadPool,
    timestamp: u64,
    state: &mut CacheState,
) {
    let mut join_set: JoinSet<(Hash, CacheEntry)> = JoinSet::new();

    for &deep_hash in state.new_entries.iter() {
        let cached_file_path = cache_path.join(format!("{:x}", deep_hash));

        join_set.spawn(thread_pool, move || {
            let content = std::fs::read(&cached_file_path).unwrap();

            let output_hash = hash_bytes(&content);
            let output_size = u32::try_from(content.len()).unwrap();

            (
                deep_hash,
                CacheEntry {
                    last_used_timestamp: timestamp,
                    output_hash,
                    output_size,
                },
            )
        });
    }

    while let Some((deep_hash, cache_entry)) = join_set.join_next() {
        assert!(state.entries.insert(deep_hash, cache_entry,).is_none());
    }
}

// This won't hopefully be reached easily in a sane project.
//
// But we still don't want the user to du their project directory and see that it grew to 10GB.
//
// Just keep it at a generous limit that won't shock the user when they check the size.
//
// Another consideration for this might be the number of files since having too many files might
// make the cache system slower, this should be explored if we find the cache operations to take too much time.
const MAX_CACHE_SIZE: u64 = 1 << 30;

// Delete the least recently used entries, along with their cached files,
// until the total size of the cached outputs drops back under `max_cache_size`.
fn spawn_prune(
    cache_path: &Path,
    thread_pool: &ThreadPool,
    join_set: &mut JoinSet<()>,
    entries: &mut BTreeMap<Hash, CacheEntry>,
    max_cache_size: u64,
) {
    let total_size: u64 = entries
        .values()
        .map(|entry| u64::from(entry.output_size))
        .sum();

    if total_size <= max_cache_size {
        return;
    }

    // order so the first item is the one with the most recent last_used_timestamp value.
    // we pop from this list to remove entries so we will be removing the oldest entry first.
    let mut sorted: Vec<(u64, Hash)> = entries
        .iter()
        .map(|(deep_hash, entry)| (entry.last_used_timestamp, *deep_hash))
        .collect();

    sorted.sort_unstable_by(|a, b| b.0.cmp(&a.0));

    let mut size = total_size;

    while size > max_cache_size {
        let (_, deep_hash) = sorted.pop().unwrap();

        let entry = entries.remove(&deep_hash).unwrap();

        size -= u64::from(entry.output_size);

        let cached_file_path = cache_path.join(format!("{:x}", deep_hash));
        join_set.spawn(thread_pool, move || {
            std::fs::remove_file(&cached_file_path).unwrap();
        });
    }
}

fn store_state_file(cache_path: &Path, entries: &BTreeMap<Hash, CacheEntry>) {
    let state_file_path = cache_path.join("cache_state");

    let mut out = Vec::new();
    serialize_state_file(entries, &mut out);

    let tmp_path = format!("{}.tmp", state_file_path.display());
    std::fs::write(&tmp_path, &out).unwrap();
    std::fs::rename(&tmp_path, &state_file_path).unwrap();
}

fn spawn_delete_wal(wal_path: &Path, thread_pool: &ThreadPool, join_set: &mut JoinSet<()>) {
    let dir = match std::fs::read_dir(wal_path) {
        Ok(d) => d,
        Err(e) => {
            assert!(e.kind() == IoErrorKind::NotFound);
            return;
        }
    };

    for entry in dir {
        let entry = entry.unwrap();
        let path = entry.path();

        join_set.spawn(thread_pool, move || {
            std::fs::remove_file(path).unwrap();
        });
    }
}

// - Prune the state if size is too big.
// - Merge new entries with existing ones. Need to collect more info per new entry to do this.
// - Store state file.
// - Delete all wal files.
pub fn finish(mut state: CacheState, cache_path: &Path, thread_pool: &ThreadPool, timestamp: u64) {
    let wal_path = cache_path.join("wal");

    let mut join_set: JoinSet<()> = JoinSet::new();

    // Doing all this spawns in parallel will likely corrupt everything in a case where we crash.
    // We don't really care about this much since this should be very rare.
    //
    // The main goal of the WAL mechanism is to not lose the progress when one of the build artifacts
    // fails a compile or when a test fails. These failures cause a crash BEFORE we call this finish function so
    // we feel safe while we are in this function.

    spawn_prune(
        cache_path,
        thread_pool,
        &mut join_set,
        &mut state.entries,
        MAX_CACHE_SIZE,
    );

    spawn_delete_wal(&wal_path, thread_pool, &mut join_set);

    // merge AFTER pruning so we never prune the new entries.
    merge_new_entries(cache_path, thread_pool, timestamp, &mut state);

    store_state_file(cache_path, &state.entries);

    join_set.join_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread_pool::{JoinSet, ThreadPool};
    use std::path::PathBuf;

    fn make_cache_dir() -> PathBuf {
        let path = PathBuf::from(format!(
            "/tmp/opencode/zxbs-tests/{}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
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

    // Runs the pruning and waits for the scheduled file deletions to finish.
    fn prune(cache_path: &Path, entries: &mut BTreeMap<Hash, CacheEntry>, max_cache_size: u64) {
        let thread_pool = ThreadPool::new(2);
        let mut join_set = JoinSet::new();

        spawn_prune(
            cache_path,
            &thread_pool,
            &mut join_set,
            entries,
            max_cache_size,
        );

        join_set.join_all();
    }

    #[test]
    fn prune_keeps_everything_when_at_or_under_the_limit() {
        let cache_path = make_cache_dir();

        // 3 entries * 10 bytes = 30 bytes total, exactly at the limit.
        let (mut entries, files) = setup_entries(&cache_path, 10, &[(1, 100), (2, 200), (3, 300)]);

        prune(&cache_path, &mut entries, 30);

        assert_eq!(entries.len(), 3);
        for file in files.iter() {
            assert!(
                file.exists(),
                "{} should not have been deleted",
                file.display()
            );
        }

        std::fs::remove_dir_all(&cache_path).unwrap();
    }

    #[test]
    fn prune_evicts_the_oldest_entries_first() {
        let cache_path = make_cache_dir();

        // 4 entries * 100 bytes = 400 bytes total. The newest entry is first in
        // the list to make sure the code does not just trust insertion order.
        let (mut entries, files) =
            setup_entries(&cache_path, 100, &[(4, 400), (2, 200), (1, 100), (3, 300)]);

        // Evicting the two oldest (200 bytes) gets us down to 200 <= 250.
        prune(&cache_path, &mut entries, 250);

        assert_eq!(entries.len(), 2);
        let remaining_timestamps: Vec<u64> = entries
            .values()
            .map(|entry| entry.last_used_timestamp)
            .collect();
        assert_eq!(remaining_timestamps, vec![300, 400]);

        // The files of the two oldest entries must be gone from disk, too.
        assert!(
            !files[1].exists(),
            "second oldest entry's file was not deleted"
        );
        assert!(!files[2].exists(), "oldest entry's file was not deleted");
        assert!(files[0].exists(), "newest entry's file was deleted");
        assert!(files[3].exists(), "second newest entry's file was deleted");

        std::fs::remove_dir_all(&cache_path).unwrap();
    }

    #[test]
    fn prune_stops_as_soon_as_it_is_under_the_limit() {
        let cache_path = make_cache_dir();

        // 4 entries * 100 bytes = 400 bytes total.
        let (mut entries, files) =
            setup_entries(&cache_path, 100, &[(1, 100), (2, 200), (3, 300), (4, 400)]);

        // Evicting only the oldest entry gets us to 300, which is not over the
        // limit, so exactly one entry must be evicted.
        prune(&cache_path, &mut entries, 300);

        assert_eq!(entries.len(), 3);
        assert!(!entries.contains_key(&1), "oldest entry was not evicted");
        assert!(!files[0].exists(), "oldest entry's file was not deleted");

        std::fs::remove_dir_all(&cache_path).unwrap();
    }
}
