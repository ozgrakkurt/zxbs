pub fn check_name(name_str: &str, err_ctx: &str) {
    let name = name_str.as_bytes();
    assert!(!name.is_empty(), "Invalid name: {} while processing {}", name_str, err_ctx);
    assert!(
        name[0] >= b'a' && name[0] <= b'z',
        "invalid name: {} while processing {}",
        name_str,
        err_ctx,
    );
    for &byte in name {
        let is_char = byte >= b'a' && byte <= b'z';
        let is_digit = byte.is_ascii_digit();
        let is_underscore = byte == b'_';
        assert!(
            is_char || is_digit || is_underscore,
            "invalid name: {} while processing {}",
            name_str,
            err_ctx,
        );
    }
}
