use crate::hash::{Hash, HashMap};

struct Entry {
    last_used_timestamp: u64,
    output_hash: Hash,
}

pub struct CacheState {
    // map deep hash to Entry
    entries: HashMap<Hash, Entry>,
}

impl CacheState {
    pub fn new() -> CacheState {
        CacheState {
            entries: HashMap::default(),
        }
    }

    pub fn deserialize(data: &[u8]) -> CacheState {
        if data.len() == 0 {
            return CacheState::new();
        }

        const HASH_SIZE: usize = std::mem::size_of::<Hash>();
        const ROW_SIZE: usize = HASH_SIZE * 2 + 8;

        assert!(data.len() % ROW_SIZE == 0);

        let mut entries = HashMap::default();
        entries.reserve(data.len() / ROW_SIZE);

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

        CacheState { entries }
    }

    pub fn serialize(&self, out: &mut Vec<u8>) {
        for (deep_hash, entry) in self.entries.iter() {
            out.extend_from_slice(&deep_hash.to_le_bytes());
            out.extend_from_slice(&entry.last_used_timestamp.to_le_bytes());
            out.extend_from_slice(&entry.output_hash.to_le_bytes());
        }
    }

    pub fn update_last_used_timestamp(&mut self, deep_hash: Hash, timestamp: u64) {
        self.entries
            .get_mut(&deep_hash)
            .unwrap()
            .last_used_timestamp = timestamp;
    }

    pub fn is_cached(&self, deep_hash: Hash) -> bool {
        self.entries.contains_key(&deep_hash)
    }

    pub fn insert(&mut self, deep_hash: Hash, output_hash: Hash, timestamp: u64) {
        self.entries.insert(
            deep_hash,
            Entry {
                last_used_timestamp: timestamp,
                output_hash,
            },
        );
    }
}
