use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
pub enum Subcommand {
    RunBinary {
        entry_name: String,
        bin_name: String,
        args: Vec<String>,
    },
    TestAll,
    Test {
        entry_name: String,
        test_name: String,
    },
    Build,
    Fmt,
}

#[derive(Debug, PartialEq)]
pub struct Config {
    pub cxx: &'static str,
    pub ar: &'static str,
    pub clang_scan_deps: &'static str,
    pub cxxflags: &'static [&'static str],
    pub ldflags: &'static [&'static str],
    pub build_path: &'static Path,
    pub src_path: &'static Path,
    pub time_trace: bool,
}

fn parse_bool_arg(args: &mut HashMap<String, String>, name: &str, default: bool) -> bool {
    let positive = format!("--{}", name);
    let negative = format!("--no-{}", name);

    match args.remove(&positive) {
        Some(v) => {
            assert_eq!(
                v.as_str(),
                "",
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
                v.as_str(),
                "",
                "expected bool flag `{}`` to not have `--flag=<value>` form",
                name,
            );
            return false;
        }
        None => {}
    }

    default
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
    build_path: &'static Path,
    cxx_time_trace: bool,
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

    if cfg.debug_info {
        ldflags.push("-g1");
        ldflags.push("-gdwarf32");
        ldflags.push("-gdwarf-5");
        ldflags.push("-fasynchronous-unwind-tables");
        ldflags.push("-DZX_DEBUG_INFO");
    } else {
        ldflags.push("-fno-unwind-tables");
        ldflags.push("-fno-asynchronous-unwind-tables");
        ldflags.push("-Wl,-s");
    }

    ldflags
}

fn leak_str(s: String) -> &'static str {
    s.leak()
}

fn parse_subcommand_arg(arg: &str) -> (&str, &str) {
    let parts: Vec<_> = arg.split(':').collect();
    assert_eq!(parts.len(), 2, "invalid subcommand argument: `{}`", arg);
    check_name(parts[0]);
    check_name(parts[1]);
    (parts[0], parts[1])
}

impl Config {
    fn parse_subcommand(
        flags_is_empty: bool,
        subcommand: &str,
        args: &mut std::env::Args,
    ) -> Subcommand {
        match subcommand {
            "run" => {
                let (entry_name, bin_name) = parse_subcommand_arg(
                    args.next().expect("Expected argument for `run` sucommand."),
                );
                Subcommand::RunBinary {
                    entry_name,
                    bin_name,
                    args: args.collect::<Vec<String>>(),
                }
            }
            "test" => {
                if let Some(arg) = args.next() {
                    let (entry_name, test_name) = parse_subcommand_arg(arg);
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
                    flags_is_empty,
                    "expected no flags while running the `fmt` subcommand"
                );
                assert!(
                    args.next().is_none(),
                    "Didn't expect arguments for `fmt` subcommand"
                );
                Subcommand::Fmt
            }
            sc => panic!("unexpected subcommand name: `{}`", sc),
        }
    }

    // returns parsed flag map and the subcommand name
    fn parse_flags(args: &mut std::env::Args) -> (HashMap<String, String>, String) {
        let mut flags = HashMap::<String, String>::new();

        while let Some(arg) = args.next() {
            if !arg.starts_with("--") {
                return (flags, arg);
            }

            let parts = arg.split('=').collect::<Vec<&str>>();

            match parts.len() {
                1 => {
                    if flags.insert(arg.clone(), "".to_owned()).is_some() {
                        panic!("Duplicate argument: {}", arg);
                    }
                }
                2 => {
                    if flags
                        .insert(parts[0].to_owned(), parts[1].to_owned())
                        .is_some()
                    {
                        panic!("Duplicate argument: {}", parts[0]);
                    }
                }
                _ => {
                    panic!("Invalid CLI argument: {}", arg);
                }
            }
        }

        panic!("Expected a subcommand.");
    }

    pub fn parse_config(mut flags: HashMap<String, String>) -> Config {
        let cxx = flags.remove("--cxx").map(leak_str).unwrap_or("clang++");
        let ar = flags.remove("--ar").map(leak_str).unwrap_or("llvm-ar");
        let clang_scan_deps = flags
            .remove("--clang-scan-deps")
            .map(leak_str)
            .unwrap_or("clang-scan-deps");
        let opt_level = flags.remove("--opt-level").map(leak_str).unwrap_or("0");
        let dev_mode = get_bool_arg(&mut flags, "dev-mode", true);
        let debug_info = get_bool_arg(&mut flags, "debug-info", true);
        let fuzz = get_bool_arg(&mut flags, "fuzz", false);
        let src_dir = flags.remove("--src-dir").map(leak_str).unwrap_or("src");
        let build_dir: &'static str = flags.remove("--build-dir").map(leak_str).unwrap_or("build");
        let time_trace = get_bool_arg(&mut flags, "time-trace", false);
        let cxx_time_trace = get_bool_arg(&mut flags, "cxx-time-trace", false);

        if !flags.is_empty() {
            eprintln!("Found unexpected flags: ");
            for arg in flags.keys() {
                eprintln!("{}", arg);
            }
            panic!("It is not allowed to pass unexpected flags, aborting build...");
        }

        let current_path = std::env::current_dir().unwrap();
        let src_path = make_path_absolute(&current_path, Path::new(src_dir)).leak();
        let build_path = make_path_absolute(&current_path, Path::new(build_dir)).leak();

        let flag_cfg = FlagCfg {
            opt_level: opt_level.to_owned(),
            dev_mode,
            debug_info,
            fuzz,
            build_path,
            cxx_time_trace,
        };
        let cxxflags = make_cxxflags(&flag_cfg).leak();
        let ldflags = make_ldflags(&flag_cfg).leak();

        Config {
            cxx: cxx.to_owned().leak(),
            ar: ar.to_owned().leak(),
            clang_scan_deps: clang_scan_deps.to_owned().leak(),
            build_path,
            src_path,
            cxxflags,
            ldflags,
            time_trace,
        }
    }

    fn parse_cli() -> (Config, Subcommand) {
        let mut args = std::env::args();
        args.next().unwrap();

        let (mut flags, subcommand_name) = parse_flags(&mut args);

        let subcommand = parse_subcommand(flags.is_empty(), subcommand_name, &mut args);

        let config = parse_config(&mut flags);

        (config, subcommand)
    }
}
