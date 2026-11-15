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
    Ok(unsafe { ptr::read_unaligned(content.as_ptr().add(offs) as *const T) })
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
//
// We will merge new_entries in the state with existing ones, do pruning based on total size and
// delete the wal files when storing the state.

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

    let checksum = hash_bytes(out.as_slice()[start_offset..]);
    out.extend_from_slice(&checksum.to_le_bytes());
}

fn deserialize_state_file(content: &[u8]) -> BTreeMap<Hash, CacheEntry> {
    let mut offset: usize = 0;

    let magic = read_bytes::<u64>(content, &mut offset).unwrap();

    assert!(magic == WAL_FILE_MAGIC);

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

    Ok(out)
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

    let checksum = hash_bytes(out.as_slice()[start_offset..]);
    out.extend_from_slice(&checksum.to_le_bytes());
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
    state: &mut CacheState,
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

        let name = entry.file_name().unwrap().to_str().unwrap();
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
                Ok(v) => {
                    let hash = hash_bytes(&v);

                    if hash != wal_entry.output_hash {
                        return None;
                    }

                    if v.len() != wal_entry.output_size {
                        return None;
                    }
                }
                Err(e) => {
                    assert!(e.kind() == IoErrorKind::NotFound);
                    return None;
                }
            };

            Some((
                deep_hash,
                CacheEntry {
                    last_used_timestamp: timetamp,
                    output_size: wal_entry.output_size,
                    output_hash: wal_entry.output_hash,
                },
            ))
        });
    }

    while let Some(res) = join_set.join_next() {
        if let Some((deep_hash, cache_entry)) = res {
            state.insert(deep_hash, cache_entry);
        }
    }
}

// - Create the <cache-path> and <cache-path>/wal directories if they don't exist.
// - Load the state file.
// - Load and validate wal files.
// - Merge state_file with state coming from wal files.
// - Delete all wal files.
pub fn load(cache_path: &Path, thread_pool: &ThreadPool, timestamp: u64) -> CacheState {
    let wal_path = cache_path.join("wal");

    if !std::fs::exists(cache_path).unwrap() {
        std::fs::create_dir_all(cache_path).unwrap();
        std::fs::create_dir(&wal_path);

        return CacheState::empty();
    }

    let state_file_path = cache_path.join("cache_state");

    let mut state: CacheState = match std::fs::read(&state_file_path) {
        Ok(v) => deserialize_cache_state(&v),
        Err(e) => {
            assert!(e.kind() == IoErrorKind::NotFound);
            CacheState::empty()
        }
    };

    load_wal(cache_path, &wal_path, thread_pool, timestamp, &mut state);

    state
}

// - Prune the state if size is too big.
// - Merge new entries with existing ones. Need to collect more info per new entry to do this.
// - Store state file.
// - Delete all wal files.
pub fn finish(state: CacheState, cache_path: &Path, timestamp: u64) {}
