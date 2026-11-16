use zxbs::common::check_name;

#[test]
fn test_valid_names() {
    for name in ["a", "abc", "a1", "a_b", "z9_", "a_very_long_name_123"] {
        check_name(name).unwrap();
    }
}

#[test]
fn test_empty_name_is_invalid() {
    assert!(check_name("").is_err());
}

#[test]
fn test_name_starting_with_a_digit_is_invalid() {
    assert!(check_name("1a").is_err());
}

#[test]
fn test_name_starting_with_an_underscore_is_invalid() {
    assert!(check_name("_a").is_err());
}

#[test]
fn test_uppercase_letters_are_invalid() {
    assert!(check_name("aB").is_err());
}

#[test]
fn test_punctuation_is_invalid() {
    assert!(check_name("a-b").is_err());
}

#[test]
fn test_non_ascii_characters_are_invalid() {
    assert!(check_name("aé").is_err());
}
