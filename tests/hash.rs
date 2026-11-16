use zxbs::hash::{Hash, UnorderedHashCombiner, hash_bytes, hash_ordered, mix_hashes};

// Combine a set of hashes into one, in the given order.
fn combine(hashes: &[Hash]) -> Hash {
    let mut combiner = UnorderedHashCombiner::new();
    for &hash in hashes.iter() {
        combiner.add(hash);
    }
    combiner.combine()
}

#[test]
fn test_unordered_hash_combiner() {
    let a = hash_bytes(b"a");
    let b = hash_bytes(b"b");
    let c = hash_bytes(b"c");

    // The combined hash doesn't depend on the order the hashes are added in.
    assert_eq!(combine(&[a, b, c]), combine(&[c, a, b]));
    assert_eq!(combine(&[a, b, c]), combine(&[b, c, a]));

    // Adding a hash that is already in the set changes nothing.
    assert_eq!(combine(&[a, b, a, b, a]), combine(&[a, b]));

    // Different sets of hashes combine to different hashes.
    assert_ne!(combine(&[a, b, c]), combine(&[a, b]));
    assert_ne!(combine(&[a]), combine(&[b]));
    assert_ne!(combine(&[a]), combine(&[]));
}

#[test]
fn test_hash_ordered() {
    // Buffer boundaries are part of the hash. Equal total bytes with a
    // different split must not collide.
    assert_ne!(hash_ordered(&[b"ab", b"c"]), hash_ordered(&[b"a", b"bc"]));

    // Ordering matters.
    assert_ne!(hash_ordered(&[b"a", b"b"]), hash_ordered(&[b"b", b"a"]));

    // An empty buffer still contributes to the hash.
    assert_ne!(hash_ordered(&[]), hash_ordered(&[b""]));

    // Hashing the same buffers always gives the same result.
    assert_eq!(
        hash_ordered(&[b"foo", b"bar"]),
        hash_ordered(&[b"foo", b"bar"])
    );
}

#[test]
fn test_mix_hashes_is_order_sensitive() {
    let a = hash_bytes(b"a");
    let b = hash_bytes(b"b");

    assert_eq!(mix_hashes(a, b), mix_hashes(a, b));
    assert_ne!(mix_hashes(a, b), mix_hashes(b, a));
}
