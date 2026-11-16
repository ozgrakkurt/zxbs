pub fn check_name(name_str: &str) -> Result<(), ()> {
    let name = name_str.as_bytes();
    if name.is_empty() {
        return Err(());
    }
    if !(name[0] >= b'a' && name[0] <= b'z') {
        return Err(());
    }
    for &byte in name {
        let is_char = byte >= b'a' && byte <= b'z';
        let is_digit = byte.is_ascii_digit();
        let is_underscore = byte == b'_';
        if !(is_char || is_digit || is_underscore) {
            return Err(());
        }
    }
    Ok(())
}
