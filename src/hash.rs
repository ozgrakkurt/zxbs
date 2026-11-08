use std::collections::BTreeSet;
use std::hash::Hasher as _;
use std::collections::HashMap as StdHashMap;

use xxhash_rust::xxh3::Xxh3 as HashBuilder;
use xxhash_rust::xxh3::Xxh3Builder as HashMapHashBuilder;

// Use u128 unless it causes real problems, we really don't want these
// hashes to collide.
pub type Hash = u128;

pub fn hash_bytes(bytes: &[u8]) -> Hash {
    xxhash_rust::xxh3::xxh3_128(bytes)
}

// Hash combiner to combine a set of unique hashes in an order independent
// way.
//
// Addition of a duplicate hash will be silently ignored.
//
// We want to make it order independant to be more resistant to
// possible order randomization in scanning.
//
// This way, even if scan returns a different ordered output every time,
// we will still calculate same hash for same artifacts and the cache will
// work properly.
//
// One might be inclined to use addition or xor to combine hashes in a commutative
// way but these are not really good options as described here:
// https://kevinventullo.com/2018/12/24/hashing-unordered-sets-how-far-will-cleverness-take-you/
//
// This approach might be too slow, we can convert to some worse reliability method in that case.
#[derive(Clone)]
pub struct HashCombiner {
    hashes: BTreeSet<Hash>,
}

impl HashCombiner {
    pub fn new() -> HashCombiner {
        HashCombiner {
            hashes: BTreeSet::new(),
        }
    }

    pub fn add(&mut self, hash: Hash) {
        self.hashes.insert(hash);
    }

    pub fn combine(&self) -> Hash {
        let mut builder = HashBuilder::new();

        for &hash in self.hashes.iter() {
            builder.write_u128(hash);
        }

        builder.digest128()
    }
}

pub type HashMap<K, V> = StdHashMap<K, V, HashMapHashBuilder>;
