use std::collections::HashMap;

struct Entry {
    last_used_timestamp: u64,
    output_hash: u64,
}

pub struct CacheState {
    // map deep hash to Entry
    entries: HashMap<u64, Entry>,
}

impl CacheState {
    pub fn new() -> CacheState {
        CacheState {
            entries: HashMap::new(),
        }
    }

    pub fn deserialize(data: &[u8]) -> CacheState {
        if data.len() == 0 {
            return CacheState::new();
        }

        assert!(data.len() % 24 == 0);

        let mut entries = HashMap::with_capacity(data.len() / 24);

        let mut offset = 0;
        while offset < data.len() {
            let deep_hash = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
            let last_used_timestamp =
                u64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
            let output_hash =
                u64::from_le_bytes(data[offset + 16..offset + 24].try_into().unwrap());

            offset += 24;

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

    pub fn update_last_used_timestamp(&mut self, deep_hash: u64, timestamp: u64) {
        self.entries
            .get_mut(&deep_hash)
            .unwrap()
            .last_used_timestamp = timestamp;
    }

    pub fn is_cached(&self, deep_hash: u64) -> bool {
        self.entries.contains_key(&deep_hash)
    }

    pub fn insert(&mut self, deep_hash: u64, output_hash: u64, timestamp: u64) {
        self.entries.insert(
            deep_hash,
            Entry {
                last_used_timestamp: timestamp,
                output_hash,
            },
        );
    }
}
