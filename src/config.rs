use std::path::{Path, PathBuf};

use crate::common::check_name;
use crate::hash::HashMap;

#[derive(Debug, PartialEq)]
pub enum Subcommand {
    RunBinary {
        entry_name: String,
        bin_name: String,
        args: &'static [&'static str],
    },
    TestAll,
    Test {
        entry_name: String,
        test_name: String,
    },
    Build,
    Fmt,
    Clean,
}

#[derive(Debug, PartialEq)]
pub struct Config {
    pub cxx: &'static str,
    pub ar: &'static str,
    pub clang_scan_deps: &'static str,
    pub clang_format: &'static str,
    pub cxxflags: &'static [&'static str],
    pub asmflags: &'static [&'static str],
    pub ldflags: &'static [&'static str],
    pub build_path: &'static Path,
    pub build_lock_path: &'static Path,
    pub src_path: &'static Path,
    pub cache_path: &'static Path,
    pub time_trace: bool,
    pub fuzz_unlimited: bool,
}

fn parse_bool_arg(args: &mut HashMap<String, Option<String>>, name: &str, default: bool) -> bool {
    let positive = format!("--{}", name);
    let negative = format!("--no-{}", name);

    match args.remove(&positive) {
        Some(v) => {
            assert_eq!(
                v, None,
                "expected bool flag `{}`` to not have `--flag=<value>` form",
                name,
            );
            assert!(
                args.remove(&negative).is_none(),
                "both negative and positive variants of flag `{}` was found.",
                name,
            );
            return true;
        }
        None => {}
    }

    match args.remove(&negative) {
        Some(v) => {
            assert_eq!(
                v, None,
                "expected bool flag `{}` to not have `--flag=<value>` form",
                name,
            );
            return false;
        }
        None => {}
    }

    default
}

fn parse_str_arg(
    args: &mut HashMap<String, Option<String>>,
    name: &str,
    default: &'static str,
) -> &'static str {
    match args.remove(name) {
        Some(v) => match v {
            None => panic!("Expected a value for flag `{}`", name),
            Some(v) => v.leak(),
        },
        None => default,
    }
}

fn make_path_absolute(cwd: &Path, path: &Path) -> PathBuf {
    if !path.is_absolute() {
        cwd.join(path)
    } else {
        path.to_owned()
    }
}

struct FlagCfg {
    opt_level: String,
    dev_mode: bool,
    debug_info: bool,
    fuzz: bool,
    fuzz_unlimited: bool,
    build_path: &'static Path,
    cxx_time_trace: bool,
}

fn make_asmflags(cfg: &FlagCfg) -> Vec<&'static str> {
    // `.S` files are preprocessed by clang before being assembled, so the
    // preprocessor flags and defines used by `make_cxxflags` are valid here
    // too. Codegen-only flags have no effect on assembly, so they are omitted.
    let mut asmflags = vec![
        // Treat preprocessor warnings and unused command line arguments as errors.
        "-Werror",
        // Target micro architecture.
        "-march=native",
        // Use cpp standard.
        "-std=c++23",
        // disable libc
        "-nostdlib",
        "-nostdinc",
        "--target=x86_64-linux-none",
        // no pie
        "-fno-pic",
        "-fno-pie",
    ];

    if cfg.debug_info {
        asmflags.push("-g1");
        asmflags.push("-gdwarf32");
        asmflags.push("-gdwarf-5");
        asmflags.push("-DZX_DEBUG_INFO");
    }

    if cfg.dev_mode {
        asmflags.push("-DZX_DEV");
    }

    if cfg.fuzz {
        asmflags.push("-DZX_FUZZ");
    }

    if cfg.fuzz_unlimited {
        if !cfg.fuzz {
            panic!("Using --fuzz-unlimited without --fuzz doesn't make sense.");
        }
        asmflags.push("-DZX_FUZZ_UNLIMITED");
    }

    asmflags
}

fn make_cxxflags(cfg: &FlagCfg) -> Vec<&'static str> {
    let mut cxxflags = vec![
        // Warnings.
        "-Wall",
        "-Wextra",
        "-Wconversion",
        "-Wsign-conversion",
        "-Wmissing-field-initializers",
        "-Wmissing-designated-field-initializers",
        "-Wshadow",
        "-Wimplicit-fallthrough",
        // We only use C style structs so this is not an issue. This can cause bugs
        // if we ever use types with constructors?
        "-Wno-reorder-init-list",
        // This causes annoying warnings with templated code.
        // Could try to enable it and then check if there are any interesting catches later.
        "-Wno-tautological-compare",
        // warn on variable length array usage
        "-Wvla",
        // these don't seem to catch much but enable anyways
        "-Wpointer-bool-conversion",
        "-Wbool-operation",
        // make sure switch on enum has a default
        "-Wswitch-default",
        // Treat warnings as errors.
        "-Werror",
        // Target micro architecture.
        "-march=native",
        // Use cpp standard.
        "-std=c++23",
        // we don't use libc but this might effect some compiler builtins?
        "-fno-math-errno",
        // this caused compilation errors so is disabled. could be an idea to re-enable it
        "-fno-stack-protector",
        // enable linker garbage collection
        "-ffunction-sections",
        "-fdata-sections",
        // disable strict aliasing
        "-fno-strict-aliasing",
        // disable undefined behavior on signed integer overflow
        "-fno-strict-overflow",
        // signed integer overflow is defined to wrap.
        "-fwrapv",
        // Don't delete `ptr == nullptr` kind of checks. And don't interpret passing nullptr on some places as
        // undefined behavior.
        "-fno-delete-null-pointer-checks",
        // disable some c++ features
        "-fno-exceptions",
        "-fno-rtti",
        "-nostdlib++",
        "-fno-threadsafe-statics",
        "-fomit-frame-pointer",
        "-momit-leaf-frame-pointer",
        // disable libc
        "-nostdlib",
        "-nostdinc",
        "--target=x86_64-linux-none",
        "-fmodules-reduced-bmi",
        // Limit max number of errors to collect before quitting compilation.
        // Having less errors printed is nice since it means it is no necessary to scroll upwards to find
        // the first errors.
        // This might be a problem if the compilation is too slow but it shouldn't be.
        "-ferror-limit=2",
        // no pie
        "-fno-pic",
        "-fno-pie",
    ];

    cxxflags.push(format!("-O{}", cfg.opt_level).leak());

    cxxflags.push(
        format!(
            "-fprebuilt-module-path={}",
            cfg.build_path.join("mod").display()
        )
        .leak(),
    );

    if cfg.cxx_time_trace {
        cxxflags.push("-ftime-trace");
    }

    if cfg.debug_info {
        cxxflags.push("-g1");
        cxxflags.push("-gdwarf32");
        cxxflags.push("-gdwarf-5");
        cxxflags.push("-fasynchronous-unwind-tables");
        cxxflags.push("-DZX_DEBUG_INFO");
    } else {
        cxxflags.push("-fno-unwind-tables");
        cxxflags.push("-fno-asynchronous-unwind-tables");
    }

    if cfg.dev_mode {
        cxxflags.push("-DZX_DEV");
    }

    if cfg.fuzz {
        cxxflags.push("-DZX_FUZZ");
        cxxflags.push("-fsanitize-coverage=trace-pc-guard");
    }

    if cfg.fuzz_unlimited {
        if !cfg.fuzz {
            panic!("Using --fuzz-unlimited without --fuzz doesn't make sense.");
        }
        cxxflags.push("-DZX_FUZZ_UNLIMITED");
    }

    cxxflags
}

fn make_ldflags(cfg: &FlagCfg) -> Vec<&'static str> {
    let mut ldflags = vec![
        "-march=native",
        // use llvm linker
        "-fuse-ld=lld",
        // don't allow undefined references
        "-Wl,-z,defs",
        // remove unused sections
        "-Wl,--gc-sections",
        // disable some c++ features
        "-fno-exceptions",
        "-fno-rtti",
        "-nostdlib++",
        "-fno-threadsafe-statics",
        "-fomit-frame-pointer",
        "-momit-leaf-frame-pointer",
        "-static",
        // disable libc
        "-nostdlib",
        "-nostdinc",
        "--target=x86_64-linux-none",
        // no pie
        "-fno-pic",
        "-fno-pie",
    ];

    ldflags.push(format!("-O{}", cfg.opt_level).leak());

    if !cfg.debug_info {
        ldflags.push("-Wl,-s");
    }

    ldflags
}

fn leak_str(s: String) -> &'static str {
    s.leak()
}

pub fn try_parse_subcommand_arg(arg: &str) -> Result<(String, String), String> {
    let parts: Vec<_> = arg.split(':').collect();
    let map_err = |_| format!("invalid subcommand argument: `{}`", arg);
    if parts.len() != 2 {
        return Err(format!(
            "Invalid subcommand argument `{}`, expected subcommand argument to have exactly one `:` character.",
            arg
        ));
    }
    check_name(parts[0]).map_err(map_err)?;
    check_name(parts[1]).map_err(map_err)?;
    Ok((parts[0].to_owned(), parts[1].to_owned()))
}

pub fn parse_subcommand_arg(arg: &str) -> (String, String) {
    try_parse_subcommand_arg(arg).unwrap_or_else(|e| panic!("{}", e))
}

impl Config {
    pub fn parse_subcommand(
        subcommand: &str,
        args: &mut impl Iterator<Item = String>,
    ) -> Subcommand {
        match subcommand {
            "run" => {
                let arg = args
                    .next()
                    .expect("Expected argument for `run` subcommand.");
                let (entry_name, bin_name) = parse_subcommand_arg(&arg);
                Subcommand::RunBinary {
                    entry_name,
                    bin_name,
                    args: args.map(leak_str).collect::<Vec<&'static str>>().leak(),
                }
            }
            "test" => {
                if let Some(arg) = args.next() {
                    let (entry_name, test_name) = parse_subcommand_arg(&arg);
                    assert!(
                        args.next().is_none(),
                        "Didn't expect arguments after the test name in `test` subcommand"
                    );
                    Subcommand::Test {
                        entry_name,
                        test_name,
                    }
                } else {
                    Subcommand::TestAll
                }
            }
            "build" => {
                assert!(
                    args.next().is_none(),
                    "Didn't expect arguments for `build` subcommand"
                );
                Subcommand::Build
            }
            "fmt" => {
                assert!(
                    args.next().is_none(),
                    "Didn't expect arguments for `fmt` subcommand"
                );
                Subcommand::Fmt
            }
            "clean" => {
                assert!(
                    args.next().is_none(),
                    "Didn't expect arguments for `clean` subcommand"
                );
                Subcommand::Clean
            }
            sc => panic!("unexpected subcommand name: `{}`", sc),
        }
    }

    // returns parsed flag map and the subcommand name
    pub fn parse_flags(
        args: &mut impl Iterator<Item = String>,
    ) -> (HashMap<String, Option<String>>, String) {
        let mut flags = HashMap::<String, Option<String>>::default();

        while let Some(arg) = args.next() {
            if !arg.starts_with("--") {
                return (flags, arg);
            }

            let parts = arg.split('=').collect::<Vec<&str>>();

            match parts.len() {
                1 => {
                    if flags.insert(arg.clone(), None).is_some() {
                        panic!("Duplicate flag: {}", arg);
                    }
                }
                2 => {
                    if flags
                        .insert(parts[0].to_owned(), Some(parts[1].to_owned()))
                        .is_some()
                    {
                        panic!("Duplicate flag: {}", parts[0]);
                    }
                }
                _ => {
                    panic!("Invalid flag: {}", arg);
                }
            }
        }

        panic!("Expected a subcommand.");
    }

    pub fn parse_config(mut flags: HashMap<String, Option<String>>) -> Config {
        let cxx = parse_str_arg(&mut flags, "--cxx", "clang++");
        let ar = parse_str_arg(&mut flags, "--ar", "llvm-ar");
        let clang_scan_deps = parse_str_arg(&mut flags, "--clang-scan-deps", "clang-scan-deps");
        let clang_format = parse_str_arg(&mut flags, "--clang-format", "clang-format");
        let opt_level = parse_str_arg(&mut flags, "--opt-level", "0");
        let dev_mode = parse_bool_arg(&mut flags, "dev-mode", true);
        let debug_info = parse_bool_arg(&mut flags, "debug-info", true);
        let fuzz = parse_bool_arg(&mut flags, "fuzz", false);
        let fuzz_unlimited = parse_bool_arg(&mut flags, "fuzz-unlimited", false);
        let src_dir = parse_str_arg(&mut flags, "--src-dir", "src");
        let build_dir = parse_str_arg(&mut flags, "--build-dir", "build");
        let time_trace = parse_bool_arg(&mut flags, "time-trace", false);
        let cxx_time_trace = parse_bool_arg(&mut flags, "cxx-time-trace", false);

        if !flags.is_empty() {
            eprintln!("Found unexpected flags: ");
            for arg in flags.keys() {
                eprintln!("{}", arg);
            }
            panic!("It is not allowed to pass unexpected flags, aborting build...");
        }

        let current_path = std::env::current_dir().unwrap();
        let src_path = make_path_absolute(&current_path, Path::new(src_dir)).leak();

        let build_base_path = make_path_absolute(&current_path, Path::new(build_dir));
        let build_path = build_base_path.join("out").leak();
        let cache_path = build_base_path.join("cache").leak();
        let build_lock_path = build_base_path.join("lock").leak();

        let flag_cfg = FlagCfg {
            opt_level: opt_level.to_owned(),
            dev_mode,
            debug_info,
            fuzz,
            build_path,
            cxx_time_trace,
            fuzz_unlimited,
        };
        let cxxflags = make_cxxflags(&flag_cfg).leak();
        let asmflags = make_asmflags(&flag_cfg).leak();
        let ldflags = make_ldflags(&flag_cfg).leak();

        Config {
            cxx,
            ar,
            clang_scan_deps,
            clang_format,
            build_path,
            build_lock_path,
            cache_path,
            src_path,
            cxxflags,
            asmflags,
            ldflags,
            time_trace,
            fuzz_unlimited,
        }
    }

    pub fn parse_cli() -> (Config, Subcommand) {
        let mut args = std::env::args();
        args.next().unwrap();

        Self::parse_cli_args(&mut args)
    }

    // The whole CLI parsing pipeline except for reading the process arguments.
    // Kept separate from `parse_cli` so tests can drive it with their own
    // argument lists.
    pub fn parse_cli_args(args: &mut impl Iterator<Item = String>) -> (Config, Subcommand) {
        let (flags, subcommand_name) = Self::parse_flags(args);

        let subcommand = Self::parse_subcommand(&subcommand_name, args);

        let config = Self::parse_config(flags);

        if subcommand == Subcommand::TestAll && config.fuzz_unlimited {
            panic!(
                "Using `test` command without a specific target while --fuzz-unlimited is not allowed. Doing this would mean some fuzz tests will run forever and some will never run since we have limited concurrency while running tests."
            );
        }

        (config, subcommand)
    }
}
