#![no_main]

use libfuzzer_sys::fuzz_target;
use zxbs::common::{check_name, try_parse_file_name};
use zxbs::config::try_parse_subcommand_arg;
use zxbs::scan::{try_parse_deps_file, try_parse_p1689};

fn valid_name_from(data: &[u8]) -> String {
    let mut name = String::new();
    for &byte in data.iter().take(16) {
        if name.is_empty() {
            name.push(char::from(b'a' + byte % 26));
        } else {
            let c = match byte % 38 {
                0..=25 => char::from(b'a' + byte % 26),
                26..=35 => char::from(b'0' + byte % 10),
                _ => '_',
            };
            name.push(c);
        }
    }
    if name.is_empty() {
        name.push('a');
    }
    name
}

fuzz_target!(|data: &[u8]| {
    // Arbitrary input must be rejected or parsed, but never panic.
    let text = String::from_utf8_lossy(data);
    let _ = check_name(&text);
    let _ = try_parse_deps_file(&text);
    let _ = try_parse_subcommand_arg(&text);
    let _ = try_parse_file_name("cc", &text);
    let _ = try_parse_file_name(&text, &text);
    let _ = try_parse_p1689(data);

    // A valid name must roundtrip through the file name, deps file and name
    // checker.
    let name = valid_name_from(data);
    let file_name = format!("{}.cc", name);
    assert_eq!(try_parse_file_name("cc", &file_name).unwrap(), name);
    assert!(try_parse_file_name("cppm", &file_name).is_err());
    assert!(check_name(&name).is_ok());

    let deps = try_parse_deps_file(&format!("\n  {}  \n", name)).unwrap();
    assert_eq!(deps, vec![name.clone()]);

    let (entry, part) = try_parse_subcommand_arg(&format!("{}:{}", name, name)).unwrap();
    assert_eq!(entry, name);
    assert_eq!(part, name);

    // Known-good p1689 input.
    let parsed = try_parse_p1689(
        br#"{"rules":[{"provides":[{"logical-name":"a"}],"requires":[{"logical-name":"b"},{"logical-name":"c"}]}]}"#,
    )
    .unwrap();
    assert_eq!(parsed.provides, vec!["a"]);
    assert_eq!(parsed.deps, vec!["b", "c"]);
    assert!(try_parse_p1689(b"not json").is_err());
});
