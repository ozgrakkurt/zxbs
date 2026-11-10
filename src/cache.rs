use std::collections::{BTreeMap, BTreeSet};

use crate::hash::Hash;

struct Entry {
    last_used_timestamp: u64,
    output_hash: Hash,
}

pub struct CacheState {
    // map deep hash to Entry
    entries: BTreeMap<Hash, Entry>,
    // list of new cache entries' deep hashes inserted in this run of the build system.
    // we keep these seperately because they won't have output_hash yet.
    // we will calculate output_hashes of these after the build is done in a separate step.
    new_entries: BTreeSet<Hash>,
}

impl CacheState {
    pub fn new() -> CacheState {
        CacheState {
            entries: BTreeMap::default(),
            new_entries: BTreeSet::new(),
        }
    }

    pub fn deserialize(data: &[u8]) -> CacheState {
        if data.is_empty() {
            return CacheState::new();
        }

        const HASH_SIZE: usize = std::mem::size_of::<Hash>();
        const ROW_SIZE: usize = HASH_SIZE * 2 + 8;

        assert!(data.len().is_multiple_of(ROW_SIZE));

        let mut entries = BTreeMap::default();

        let mut offset = 0;
        while offset < data.len() {
            let deep_hash =
                Hash::from_le_bytes(data[offset..offset + HASH_SIZE].try_into().unwrap());
            let last_used_timestamp = u64::from_le_bytes(
                data[offset + HASH_SIZE..offset + HASH_SIZE + 8]
                    .try_into()
                    .unwrap(),
            );
            let output_hash = Hash::from_le_bytes(
                data[offset + HASH_SIZE + 8..offset + ROW_SIZE]
                    .try_into()
                    .unwrap(),
            );

            offset += ROW_SIZE;

            entries.insert(
                deep_hash,
                Entry {
                    last_used_timestamp,
                    output_hash,
                },
            );
        }

        CacheState {
            entries,
            new_entries: BTreeSet::new(),
        }
    }

    pub fn serialize(&self, out: &mut Vec<u8>) {
        for (deep_hash, entry) in self.entries.iter() {
            out.extend_from_slice(&deep_hash.to_le_bytes());
            out.extend_from_slice(&entry.last_used_timestamp.to_le_bytes());
            out.extend_from_slice(&entry.output_hash.to_le_bytes());
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
                self.new_entries.insert(deep_hash);
                None
            }
        }
    }
}
