use std::collections::BTreeSet;
use std::collections::HashMap as StdHashMap;
use std::collections::HashSet as StdHashSet;
use std::hash::Hasher as _;

use xxhash_rust::xxh3::Xxh3Default as HashBuilder;
use xxhash_rust::xxh3::Xxh3DefaultBuilder as HashMapHashBuilder;

// Use u128 unless it causes real problems, we really don't want these
// hashes to collide.
pub type Hash = u128;

pub fn hash_bytes(bytes: &[u8]) -> Hash {
    xxhash_rust::xxh3::xxh3_128(bytes)
}

// Order DEPENDENT hashing of multiple buffers into a single hash value.
pub fn hash_ordered(buffers: &[&[u8]]) -> Hash {
    let mut builder = HashBuilder::new();
    for &buf in buffers.iter() {
        builder.update(buf);
    }
    builder.digest128()
}

pub fn mix_hashes(a: Hash, b: Hash) -> Hash {
    let mut builder = HashBuilder::new();

    builder.write_u128(a);
    builder.write_u128(b);

    builder.digest128()
}

// Hash combiner to combine a set of unique hashes in an order independent
// way.
//
// Addition of a duplicate hash will be silently ignored.
//
// We want to make it order independent to be more resistant to
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
//
// Note: Keep in mind that commutative combination of hashes might not be what is wanted in some
// cases. If the items changing  "order" changes the output then commutative hashing might even be a bug.
#[derive(Clone)]
pub struct UnorderedHashCombiner {
    hashes: BTreeSet<Hash>,
}

impl UnorderedHashCombiner {
    pub fn new() -> UnorderedHashCombiner {
        UnorderedHashCombiner {
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
pub type HashSet<K> = StdHashSet<K, HashMapHashBuilder>;
