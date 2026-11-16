use zxbs::interner::Interner;

#[test]
fn test_interner_roundtrip() {
    let mut interner = Interner::new();

    let a = interner.insert("a");
    let b = interner.insert("b");

    assert_eq!(a.to_usize(), 0);
    assert_eq!(b.to_usize(), 1);
    assert_eq!(interner.len(), 2);

    // Inserting an existing string returns the original id and doesn't grow
    // the interner.
    assert_eq!(interner.insert("a"), a);
    assert_eq!(interner.insert("b"), b);
    assert_eq!(interner.len(), 2);

    // Ids map back to the exact strings they were created from.
    assert_eq!(interner.get(a), "a");
    assert_eq!(interner.get(b), "b");
}
