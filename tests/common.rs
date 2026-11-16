use zxbs::common::check_name;

#[test]
fn test_valid_names() {
    for name in ["a", "abc", "a1", "a_b", "z9_", "a_very_long_name_123"] {
        check_name(name, "test");
    }
}

#[test]
#[should_panic]
fn test_empty_name_is_invalid() {
    check_name("", "test");
}

#[test]
#[should_panic]
fn test_name_starting_with_a_digit_is_invalid() {
    check_name("1a", "test");
}

#[test]
#[should_panic]
fn test_name_starting_with_an_underscore_is_invalid() {
    check_name("_a", "test");
}

#[test]
#[should_panic]
fn test_uppercase_letters_are_invalid() {
    check_name("aB", "test");
}

#[test]
#[should_panic]
fn test_punctuation_is_invalid() {
    check_name("a-b", "test");
}

#[test]
#[should_panic]
fn test_non_ascii_characters_are_invalid() {
    check_name("aé", "test");
}
