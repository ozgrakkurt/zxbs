use std::path::Path;
use std::time::Instant;

use zxbs::base_hash::calculate_base_hash;
use zxbs::build::Build;
use zxbs::command::Command;
use zxbs::config::{Config, Subcommand};
use zxbs::hash::HashMap;
use zxbs::interner::Interner;
use zxbs::plan::plan;
use zxbs::scan;
use zxbs::thread_pool::ThreadPool;
use zxbs::timestamp::timestamp;
use zxbs::{cache, directory};

mod make_folders;
use make_folders::{make_baselib, make_iolib, make_myapp};

mod tmp_dir;
use tmp_dir::TempDir;

// The source tree contains 3 entries with a total of 51 build artifacts that
// get stored to the cache. A fully cached rebuild produces one cache load
// task per artifact.
const NUM_ARTIFACTS: usize = 51;

#[test]
fn test_smoke() {
    let folder = TempDir::new();
    let build_path = format!("{}/build", folder.as_str());
    let src_path = format!("{}/src", folder.as_str());
    std::fs::create_dir_all(&src_path).unwrap();

    // create folders/files
    make_baselib(Path::new(&src_path));
    make_iolib(Path::new(&src_path));
    make_myapp(Path::new(&src_path));

    let mut flags = HashMap::default();
    flags.insert("--build-dir".to_owned(), Some(build_path.clone()));
    flags.insert("--src-dir".to_owned(), Some(src_path.clone()));
    let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

    let thread_pool = ThreadPool::new(16);

    let mut interner = Interner::new();

    // The base hash incorporates the build configuration and the hash of this
    // binary. It must be the same for both runs below so that the cache hits
    // of the second run work.
    let base_hash = calculate_base_hash(cfg, &thread_pool);

    // First run: a full build from a cold cache.
    {
        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

        let info = scan::scan(
            cfg,
            &mut interner,
            &thread_pool,
            base_hash,
            &mut cache_state,
            ts,
        );
        info.check_subcommand(&mut interner, &Subcommand::TestAll);

        directory::setup_build_directories(cfg, &info, &interner, &thread_pool);

        let tasks = plan(
            cfg,
            &Subcommand::TestAll,
            &mut interner,
            &info,
            &mut cache_state,
            base_hash,
            ts,
        );
        assert_eq!(tasks.len(), 91);

        Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();

        cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);
    }

    // The build outputs actually exist.
    {
        let build_path = cfg.build_path;

        assert!(build_path.join("mod/base.pcm").exists());
        assert!(build_path.join("entry/base/lib/libbase.a").exists());
        assert!(build_path.join("entry/io/lib/libio.a").exists());
        assert!(build_path.join("entry/myapp/lib/libmyapp.a").exists());
        assert!(build_path.join("entry/myapp/bin/myapp").exists());
        assert!(build_path.join("entry/io/bin/direct_io_example").exists());

        for (entry_name, test_name) in [
            ("base", "memcmp"),
            ("base", "vec"),
            ("base", "arith"),
            ("io", "executor_ordering"),
            ("io", "fs"),
            ("io", "net"),
        ] {
            let stamp_path = build_path
                .join(format!("entry/{}", entry_name))
                .join("test")
                .join("@stamp@")
                .join(test_name);
            assert!(stamp_path.exists());
            // The stamp file is just the hash the test result is cached by.
            assert_eq!(std::fs::read(&stamp_path).unwrap().len(), 16);
        }
    }

    // Second run: a warm rebuild. Everything, including the scan deps results
    // and the test stamps, must come from the cache now. That means no test
    // is re-run and every task is a cache load.
    {
        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

        let info = scan::scan(
            cfg,
            &mut interner,
            &thread_pool,
            base_hash,
            &mut cache_state,
            ts,
        );

        directory::setup_build_directories(cfg, &info, &interner, &thread_pool);

        let tasks = plan(
            cfg,
            &Subcommand::TestAll,
            &mut interner,
            &info,
            &mut cache_state,
            base_hash,
            ts,
        );

        assert_eq!(tasks.len(), NUM_ARTIFACTS);

        // Cache loads now carry the same dependencies as the compile/link/run
        // tasks they replace. Every dependency must be produced by another
        // task in the warm plan, and at least one task must have one (a fully
        // warm plan where nothing depends on anything would miss the PCM and
        // object ordering edges).
        let mut produced: HashMap<&'static str, ()> = HashMap::default();
        for task in tasks.iter() {
            for &output in task.outputs.iter() {
                assert!(
                    produced.insert(interner.get(output), ()).is_none(),
                    "an artifact was produced by two tasks: `{}`",
                    interner.get(output),
                );
            }
        }

        let mut saw_dependency = false;
        for task in tasks.iter() {
            match &task.command {
                Some(Command::CacheLoad(cl)) => {
                    assert_eq!(task.outputs.len(), 1);
                    assert_ne!(cl.deep_hash, 0);
                    assert_ne!(cl.expected_hash, 0);

                    for &input in task.inputs.iter() {
                        let input = interner.get(input);
                        assert!(
                            produced.contains_key(input),
                            "cache load for `{}` depends on `{}`, which no task produces",
                            cl.output_path,
                            input,
                        );
                        saw_dependency = true;
                    }
                }
                _ => {
                    panic!(
                        "expected only cache loads in the warm rebuild, got: {}",
                        task.command.as_ref().unwrap().print(),
                    );
                }
            }
        }
        assert!(
            saw_dependency,
            "expected warm cache loads to carry dependency edges",
        );

        Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();

        cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);
    }

    // The outputs were restored from the cache.
    assert!(cfg.build_path.join("entry/myapp/bin/myapp").exists());
    assert!(
        cfg.build_path
            .join("entry/base/test/@stamp@/memcmp")
            .exists()
    );
}

// With `--asm` every library, binary and test object file gets an annotated
// assembly file next to it. Those files are restored from the cache just like
// the object files.
#[test]
fn test_asm_output() {
    let folder = TempDir::new();
    let build_path = format!("{}/build", folder.as_str());
    let src_path = format!("{}/src", folder.as_str());
    std::fs::create_dir_all(&src_path).unwrap();

    make_baselib(Path::new(&src_path));

    let mut flags = HashMap::default();
    flags.insert("--build-dir".to_owned(), Some(build_path.clone()));
    flags.insert("--src-dir".to_owned(), Some(src_path.clone()));
    flags.insert("--asm".to_owned(), None);
    flags.insert("--opt-level".to_owned(), Some("2".to_owned()));
    let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

    let thread_pool = ThreadPool::new(16);

    let mut interner = Interner::new();

    let base_hash = calculate_base_hash(cfg, &thread_pool);

    // Every artifact of the base tree that is built from a library, binary or
    // test source file.
    let objects = [
        "entry/base/lib/memcmp",
        "entry/base/lib/memcpy",
        "entry/base/bin/memcpy_bench",
        "entry/base/test/memcmp",
        "entry/base/test/vec",
        "entry/base/test/arith",
    ];

    // First run: a full build from a cold cache.
    {
        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

        let info = scan::scan(
            cfg,
            &mut interner,
            &thread_pool,
            base_hash,
            &mut cache_state,
            ts,
        );
        info.check_subcommand(&mut interner, &Subcommand::Build);

        directory::setup_build_directories(cfg, &info, &interner, &thread_pool);

        let tasks = plan(
            cfg,
            &Subcommand::Build,
            &mut interner,
            &info,
            &mut cache_state,
            base_hash,
            ts,
        );

        // 37 normal tasks plus a compile and a store for each of the 6
        // assembly files.
        assert_eq!(tasks.len(), 37 + 2 * objects.len());

        Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();

        cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);
    }

    for object in objects.iter() {
        let object_path = cfg.build_path.join(format!("{}.o", object));
        let asm_path = cfg.build_path.join(format!("{}.s", object));

        assert!(object_path.exists(), "missing {}", object_path.display());

        let asm = std::fs::read_to_string(&asm_path)
            .unwrap_or_else(|e| panic!("missing {}: {}", asm_path.display(), e));
        assert!(!asm.trim().is_empty());
        assert!(
            asm.contains(".text"),
            "{} doesn't look like assembly",
            asm_path.display(),
        );
        assert!(
            asm.contains('#'),
            "{} doesn't look like *annotated* assembly",
            asm_path.display(),
        );
    }

    // Second run: everything, including the assembly files, comes from the
    // cache.
    {
        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

        let info = scan::scan(
            cfg,
            &mut interner,
            &thread_pool,
            base_hash,
            &mut cache_state,
            ts,
        );

        directory::setup_build_directories(cfg, &info, &interner, &thread_pool);

        let tasks = plan(
            cfg,
            &Subcommand::Build,
            &mut interner,
            &info,
            &mut cache_state,
            base_hash,
            ts,
        );

        // 21 regular artifacts + 6 assembly files.
        assert_eq!(tasks.len(), 21 + objects.len());
        assert!(
            tasks
                .iter()
                .all(|task| matches!(task.command, Some(Command::CacheLoad(_)),))
        );

        Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();

        cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);
    }

    for object in objects.iter() {
        assert!(cfg.build_path.join(format!("{}.s", object)).exists());
    }

    // Module object files don't get assembly output.
    assert!(!cfg.build_path.join("mod/base.s").exists());
    assert!(!cfg.build_path.join("mod/base-vec.s").exists());
}
