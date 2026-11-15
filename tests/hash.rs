use zxbs::hash::{Hash, UnorderedHashCombiner, hash_bytes};

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
