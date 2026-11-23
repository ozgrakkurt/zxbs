use std::path::Path;

use zxbs::config::{self, Config, Subcommand};
use zxbs::hash::HashMap;

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
    assert!(!cfg.emit_asm);

    // Debug info is on by default so the linker shouldn't strip symbols.
    assert!(!has_flag(cfg.ldflags, "-Wl,-s"));
}

#[test]
fn test_asm_flag() {
    let cfg = Config::parse_config(flags(&[("--asm", None)]));
    assert!(cfg.emit_asm);

    let cfg = Config::parse_config(flags(&[("--no-asm", None)]));
    assert!(!cfg.emit_asm);
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

// `--cxx-time-trace` is passed through to the compiler flags.
#[test]
fn test_cxx_time_trace_flag() {
    let cfg = Config::parse_config(flags(&[("--cxx-time-trace", None)]));

    assert!(has_flag(cfg.cxxflags, "-ftime-trace"));

    let cfg = Config::parse_config(flags(&[]));
    assert!(!has_flag(cfg.cxxflags, "-ftime-trace"));
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

// A string flag must not appear without its value.
#[test]
#[should_panic(expected = "Expected a value for flag `--cxx`")]
fn test_str_flag_without_a_value_is_rejected() {
    Config::parse_config(flags(&[("--cxx", None)]));
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

// `parse_flags` splits the leading `--flag`/`--flag=value` arguments from the
// subcommand name that terminates the flag list.
#[test]
fn test_parse_flags() {
    let mut args = ["--dev-mode", "--build-dir=/some/build", "test", "extra"]
        .map(str::to_owned)
        .into_iter();

    let (flags, subcommand) = Config::parse_flags(&mut args);

    assert_eq!(subcommand, "test");
    assert_eq!(flags.get("--dev-mode"), Some(&None));
    assert_eq!(
        flags.get("--build-dir"),
        Some(&Some("/some/build".to_owned()))
    );
    assert_eq!(flags.len(), 2);
}

#[test]
#[should_panic(expected = "Duplicate flag")]
fn test_duplicate_bare_flags_are_rejected() {
    let mut args = ["--dev-mode", "--dev-mode", "build"]
        .map(str::to_owned)
        .into_iter();
    Config::parse_flags(&mut args);
}

#[test]
#[should_panic(expected = "Duplicate flag")]
fn test_duplicate_valued_flags_are_rejected() {
    let mut args = ["--cxx=a", "--cxx=b", "build"]
        .map(str::to_owned)
        .into_iter();
    Config::parse_flags(&mut args);
}

#[test]
#[should_panic(expected = "Invalid flag")]
fn test_flag_with_two_values_is_rejected() {
    let mut args = ["--cxx=a=b", "build"].map(str::to_owned).into_iter();
    Config::parse_flags(&mut args);
}

#[test]
#[should_panic(expected = "Expected a subcommand")]
fn test_missing_subcommand_is_rejected() {
    let mut args = ["--dev-mode"].map(str::to_owned).into_iter();
    Config::parse_flags(&mut args);
}

#[test]
fn test_parse_subcommand() {
    // `run` consumes the entry:bin argument and passes the rest to the binary.
    let sc = Config::parse_subcommand(
        "run",
        &mut ["myapp:main", "--foo"].map(str::to_owned).into_iter(),
    );
    assert_eq!(
        sc,
        Subcommand::RunBinary {
            entry_name: "myapp".to_owned(),
            bin_name: "main".to_owned(),
            args: &["--foo"],
        }
    );

    // `test` without a target runs all tests, with a target exactly one.
    let sc = Config::parse_subcommand("test", &mut std::iter::empty::<String>());
    assert_eq!(sc, Subcommand::TestAll);
    let sc = Config::parse_subcommand("test", &mut ["base:memcmp"].map(str::to_owned).into_iter());
    assert_eq!(
        sc,
        Subcommand::Test {
            entry_name: "base".to_owned(),
            test_name: "memcmp".to_owned(),
        }
    );

    // `build`, `fmt` and `clean` take no arguments.
    for (name, expected) in [
        ("build", Subcommand::Build),
        ("fmt", Subcommand::Fmt),
        ("clean", Subcommand::Clean),
    ] {
        let sc = Config::parse_subcommand(name, &mut std::iter::empty::<String>());
        assert_eq!(sc, expected);
    }
}

#[test]
#[should_panic(expected = "Expected argument for `run`")]
fn test_run_subcommand_requires_a_target() {
    Config::parse_subcommand("run", &mut std::iter::empty::<String>());
}

#[test]
#[should_panic(expected = "Didn't expect arguments after the test name")]
fn test_test_subcommand_rejects_extra_arguments() {
    Config::parse_subcommand(
        "test",
        &mut ["base:memcmp", "extra"].map(str::to_owned).into_iter(),
    );
}

#[test]
#[should_panic(expected = "Didn't expect arguments for `build` subcommand")]
fn test_build_subcommand_rejects_arguments() {
    Config::parse_subcommand("build", &mut ["extra"].map(str::to_owned).into_iter());
}

#[test]
#[should_panic(expected = "unexpected subcommand name")]
fn test_unknown_subcommand_is_rejected() {
    Config::parse_subcommand("deploy", &mut std::iter::empty::<String>());
}

// The full CLI pipeline: flags are turned into a config and the subcommand is
// parsed alongside them.
#[test]
fn test_parse_cli_args() {
    let (cfg, sc) = Config::parse_cli_args(
        &mut ["--fuzz", "--opt-level=2", "build"]
            .map(str::to_owned)
            .into_iter(),
    );

    assert_eq!(sc, Subcommand::Build);
    assert!(cfg.cxxflags.contains(&"-DZX_FUZZ"));
    assert!(cfg.cxxflags.contains(&"-O2"));
}

// Running every fuzz target forever is not a valid combination, so `test`
// without a specific target must reject --fuzz-unlimited.
#[test]
#[should_panic(expected = "--fuzz-unlimited is not allowed")]
fn test_test_all_rejects_fuzz_unlimited() {
    Config::parse_cli_args(
        &mut ["--fuzz", "--fuzz-unlimited", "test"]
            .map(str::to_owned)
            .into_iter(),
    );
}

// A specific test target is fine though: only one fuzz target runs.
#[test]
fn test_specific_test_allows_fuzz_unlimited() {
    let (_, sc) = Config::parse_cli_args(
        &mut ["--fuzz", "--fuzz-unlimited", "test", "base:memcmp"]
            .map(str::to_owned)
            .into_iter(),
    );

    assert_eq!(
        sc,
        Subcommand::Test {
            entry_name: "base".to_owned(),
            test_name: "memcmp".to_owned(),
        }
    );
}
