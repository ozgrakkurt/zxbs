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

// The source tree contains 3 entries with a total of 51 build artifacts that
// get stored to the cache. A fully cached rebuild produces one cache load
// task per artifact.
const NUM_ARTIFACTS: usize = 51;

#[test]
fn test_smoke() {
    let folder = format!("/tmp/opencode/zxbs-tests/{}", rand::random::<u128>());
    let build_path = format!("{}/build", folder);
    let src_path = format!("{}/src", folder);
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
                .join("stamp")
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
        for task in tasks.iter() {
            match &task.command {
                Some(Command::CacheLoad(cl)) => {
                    assert!(task.inputs.is_empty());
                    assert_eq!(task.outputs.len(), 1);
                    assert_ne!(cl.deep_hash, 0);
                    assert_ne!(cl.expected_hash, 0);
                }
                _ => {
                    panic!(
                        "expected only cache loads in the warm rebuild, got: {}",
                        task.command.as_ref().unwrap().print(),
                    );
                }
            }
        }

        Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();

        cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);
    }

    // The outputs were restored from the cache.
    assert!(cfg.build_path.join("entry/myapp/bin/myapp").exists());
    assert!(cfg.build_path.join("entry/base/test/stamp/memcmp").exists());

    std::fs::remove_dir_all(&folder).unwrap();
}
