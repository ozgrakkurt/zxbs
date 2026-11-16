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

fn flags(entries: &[(&str, Option<&str>)]) -> HashMap<String, Option<String>> {
    entries
        .iter()
        .map(|&(name, value)| (name.to_owned(), value.map(str::to_owned)))
        .collect()
}

fn has_flag(flags: &[&str], flag: &str) -> bool {
    flags.contains(&flag)
}

#[test]
fn test_default_flag_values() {
    let cfg = Config::parse_config(HashMap::default());

    assert!(has_flag(cfg.cxxflags, "-DZX_DEV"));
    assert!(has_flag(cfg.cxxflags, "-g1"));
    assert!(!has_flag(cfg.cxxflags, "-DZX_FUZZ"));
    assert!(!has_flag(cfg.cxxflags, "-DZX_FUZZ_UNLIMITED"));
    assert!(!cfg.fuzz_unlimited);

    // Debug info is on by default so the linker shouldn't strip symbols.
    assert!(!has_flag(cfg.ldflags, "-Wl,-s"));
}

#[test]
fn test_negative_bool_flags() {
    let cfg = Config::parse_config(flags(&[("--no-dev-mode", None), ("--no-debug-info", None)]));

    assert!(!has_flag(cfg.cxxflags, "-DZX_DEV"));

    assert!(!has_flag(cfg.cxxflags, "-g1"));
    assert!(has_flag(cfg.cxxflags, "-fno-unwind-tables"));
    assert!(has_flag(cfg.ldflags, "-Wl,-s"));
}

#[test]
fn test_opt_level_is_used_for_compiling_and_linking() {
    let cfg = Config::parse_config(flags(&[("--opt-level", Some("3"))]));

    assert!(has_flag(cfg.cxxflags, "-O3"));
    assert!(has_flag(cfg.ldflags, "-O3"));
}

#[test]
fn test_fuzz_flags() {
    let cfg = Config::parse_config(flags(&[("--fuzz", None)]));
    assert!(has_flag(cfg.cxxflags, "-DZX_FUZZ"));
    assert!(has_flag(cfg.cxxflags, "-fsanitize-coverage=trace-pc-guard"));
    assert!(!has_flag(cfg.cxxflags, "-DZX_FUZZ_UNLIMITED"));
    assert!(!cfg.fuzz_unlimited);

    let cfg = Config::parse_config(flags(&[("--fuzz", None), ("--fuzz-unlimited", None)]));
    assert!(has_flag(cfg.cxxflags, "-DZX_FUZZ_UNLIMITED"));
    assert!(cfg.fuzz_unlimited);
}

#[test]
#[should_panic(expected = "fuzz-unlimited")]
fn test_fuzz_unlimited_requires_fuzz() {
    Config::parse_config(flags(&[("--fuzz-unlimited", None)]));
}

#[test]
fn test_relative_paths_are_resolved_from_cwd() {
    let cwd = std::env::current_dir().unwrap();
    let cfg = Config::parse_config(flags(&[
        ("--src-dir", Some("relative-src")),
        ("--build-dir", Some("relative-build")),
    ]));

    assert_eq!(cfg.src_path.to_path_buf(), cwd.join("relative-src"));
    assert_eq!(cfg.build_path.to_path_buf(), cwd.join("relative-build/out"));
    assert_eq!(
        cfg.cache_path.to_path_buf(),
        cwd.join("relative-build/cache")
    );
}

#[test]
#[should_panic(expected = "both negative and positive")]
fn test_conflicting_bool_flags_are_rejected() {
    Config::parse_config(flags(&[("--dev-mode", None), ("--no-dev-mode", None)]));
}

#[test]
#[should_panic(expected = "expected bool flag")]
fn test_bool_flag_with_a_value_is_rejected() {
    Config::parse_config(flags(&[("--dev-mode", Some("1"))]));
}

#[test]
#[should_panic(expected = "unexpected flags")]
fn test_unknown_flags_are_rejected() {
    Config::parse_config(flags(&[("--not-a-real-flag", None)]));
}

#[test]
#[should_panic]
fn test_parse_subcommand_arg_without_separator() {
    config::parse_subcommand_arg("myapp");
}

#[test]
#[should_panic]
fn test_parse_subcommand_arg_with_extra_separator() {
    config::parse_subcommand_arg("myapp:main:extra");
}

#[test]
#[should_panic]
fn test_parse_subcommand_arg_with_invalid_name() {
    config::parse_subcommand_arg("myapp:Main");
}
