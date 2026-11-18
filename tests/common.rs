use zxbs::common::{check_name, make_tmp_path, try_parse_file_name};

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

#[test]
fn test_try_parse_file_name() {
    assert_eq!(
        try_parse_file_name("cppm", "my_mod.cppm").unwrap(),
        "my_mod"
    );
    assert_eq!(try_parse_file_name("cc", "main.cc").unwrap(), "main");

    // Wrong extension.
    assert!(try_parse_file_name("cc", "my_mod.cppm").is_err());
    // Invalid name part.
    assert!(try_parse_file_name("cppm", "My_mod.cppm").is_err());
    assert!(try_parse_file_name("cppm", "1my_mod.cppm").is_err());
    // Not exactly one `.` character.
    assert!(try_parse_file_name("cppm", "my_mod").is_err());
    assert!(try_parse_file_name("cppm", "my.mod.cppm").is_err());
    assert!(try_parse_file_name("cppm", ".cppm").is_err());
}

// The error messages must mention the offending file name so the user can
// find it in their tree.
#[test]
fn test_try_parse_file_name_errors_mention_the_name() {
    let err = try_parse_file_name("cc", "my_mod.cppm").unwrap_err();
    assert!(err.contains("my_mod.cppm"), "unexpected error: {}", err);

    let err = try_parse_file_name("cppm", "My_mod.cppm").unwrap_err();
    assert!(err.contains("My_mod"), "unexpected error: {}", err);

    let err = try_parse_file_name("cppm", "my_mod").unwrap_err();
    assert!(err.contains("my_mod"), "unexpected error: {}", err);
}

#[test]
fn test_make_tmp_path() {
    let a = make_tmp_path("build/out/some_file");
    let b = make_tmp_path("build/out/some_file");

    // The tmp path hangs off the output path, is tagged as temporary and is
    // unique so concurrent tasks never pick the same one.
    assert!(a.starts_with("build/out/some_file."), "{}", a);
    assert!(a.ends_with(".tmp"), "{}", a);
    assert_ne!(a, b);

    let path = make_tmp_path("name_with_no_slash");
    assert!(path.starts_with("name_with_no_slash."), "{}", path);
    assert!(path.ends_with(".tmp"), "{}", path);
}
