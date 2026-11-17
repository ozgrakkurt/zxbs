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

pub fn try_parse_file_name<'name>(
    expected_extension: &str,
    file_name: &'name str,
) -> Result<&'name str, String> {
    let parts: Vec<_> = file_name.split(".").collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid file name: {}, expected the file name to contain exactly one `.` character.",
            file_name,
        ));
    }
    let name = parts[0];
    let extension = parts[1];

    check_name(name).map_err(|_| {
        format!(
            "Unexpected name part of file name: {}, expected it to be a valid name.",
            name
        )
    })?;

    if expected_extension != extension {
        return Err(format!(
            "Unexpected extension on filename: {}, expected extension: {}",
            file_name, expected_extension,
        ));
    }

    Ok(name)
}
