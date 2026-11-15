use std::path::Path;

use zxbs::config::{self, Config};
use zxbs::hash::HashMap;

#[test]
fn test_empty() {
    Config::parse_config(HashMap::default());
}

#[test]
fn test_simple() {
    let mut flags = HashMap::default();
    flags.insert("--build-dir".to_owned(), Some("/build123".to_owned()));
    flags.insert("--src-dir".to_owned(), Some("/src123".to_owned()));
    let cfg = Config::parse_config(flags);

    assert_eq!(cfg.build_path, Path::new("/build123/out"));
    assert_eq!(cfg.cache_path, Path::new("/build123/cache"));
    assert_eq!(cfg.src_path, Path::new("/src123"));

    assert_eq!(cfg.cxx, "clang++");
    assert_eq!(cfg.ar, "llvm-ar");
    assert_eq!(cfg.clang_scan_deps, "clang-scan-deps");
    assert_eq!(cfg.clang_format, "clang-format");
}

#[test]
fn test_parse_subcommand_arg() {
    let (entry_name, part_name) = config::parse_subcommand_arg("myapp:main");
    assert_eq!(entry_name, "myapp");
    assert_eq!(part_name, "main");
}
