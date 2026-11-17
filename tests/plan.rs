#![allow(clippy::too_many_arguments)]

use std::path::Path;
use std::time::Instant;

use zxbs::build::{Build, Task};
use zxbs::cache;
use zxbs::command::Command;
use zxbs::config::{Config, Subcommand};
use zxbs::directory;
use zxbs::hash::{Hash, HashMap, hash_bytes};
use zxbs::interner::{Interner, Str};
use zxbs::plan::{self, make_module_artifact_name, should_run_binary, should_run_test};
use zxbs::scan::{self, BuildInfo};
use zxbs::thread_pool::ThreadPool;
use zxbs::timestamp::timestamp;

mod make_folders;
use make_folders::{make_baselib, make_chain, make_diamond, make_iolib, make_myapp, make_util_app};

mod tmp_dir;
use tmp_dir::TempDir;

// The source tree used by the plan tests contains 3 entries. The tasks
// emitted for each entry break down as:
//
// base:  5 modules * (1 compile + 2 stores) + 2 lib objs * 2 + 1 archive * 2
//        + 3 tests * 6 + 1 bin * 4                          = 43
// io:    5 modules * 3 + 0 lib objs * 2 + 1 archive * 2
//        + 3 tests * 6 + 1 bin * 4                          = 39
// myapp: 1 module * 3 + 0 lib objs * 2 + 1 archive * 2
//        + 0 tests * 6 + 1 bin * 4                          = 9
//
// for a total of 91 tasks. 51 distinct artifacts get stored to the cache
// (91 - 40 non-store commands).
const NUM_TASKS: usize = 91;
const NUM_STORED_ARTIFACTS: usize = 51;

fn make_config(folder: &str) -> &'static Config {
    let build_dir = format!("{}/build", folder);
    let src_dir = format!("{}/src", folder);
    std::fs::create_dir_all(&src_dir).unwrap();

    let mut flags = HashMap::default();
    flags.insert("--build-dir".to_owned(), Some(build_dir));
    flags.insert("--src-dir".to_owned(), Some(src_dir));
    Box::leak(Box::new(Config::parse_config(flags)))
}

fn strs(interner: &Interner, list: &[Str]) -> Vec<&'static str> {
    list.iter().map(|&s| interner.get(s)).collect()
}

fn as_strs(list: &[String]) -> Vec<&str> {
    list.iter().map(|s| s.as_str()).collect()
}

#[test]
fn test_should_run_test() {
    let sc = Subcommand::TestAll;
    assert!(should_run_test(&sc, "a", "b"));
    assert!(!should_run_test(&Subcommand::Build, "a", "b"));
    assert!(!should_run_test(&Subcommand::Fmt, "a", "b"));
    assert!(!should_run_test(
        &Subcommand::RunBinary {
            entry_name: "a".to_owned(),
            bin_name: "b".to_owned(),
            args: &[],
        },
        "a",
        "b",
    ));

    let sc = Subcommand::Test {
        entry_name: "a".to_owned(),
        test_name: "t".to_owned(),
    };
    assert!(should_run_test(&sc, "a", "t"));
    assert!(!should_run_test(&sc, "a", "x"));
    assert!(!should_run_test(&sc, "b", "t"));
}

#[test]
fn test_should_run_binary() {
    let sc = Subcommand::RunBinary {
        entry_name: "a".to_owned(),
        bin_name: "b".to_owned(),
        args: &["--some-arg"],
    };
    assert_eq!(
        should_run_binary(&sc, "a", "b").map(|v| v.to_vec()),
        Some(vec!["--some-arg"]),
    );
    assert_eq!(should_run_binary(&sc, "a", "c"), None);
    assert_eq!(should_run_binary(&sc, "c", "b"), None);

    assert_eq!(should_run_binary(&Subcommand::Build, "a", "b"), None);
    assert_eq!(should_run_binary(&Subcommand::TestAll, "a", "b"), None);
    assert_eq!(
        should_run_binary(
            &Subcommand::Test {
                entry_name: "a".to_owned(),
                test_name: "b".to_owned(),
            },
            "a",
            "b",
        ),
        None,
    );
}

#[test]
fn test_empty() {
    let folder = TempDir::new();
    let cfg = make_config(folder.as_str());

    let thread_pool = ThreadPool::new(4);

    let ts = timestamp();
    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

    let mut interner = Interner::new();

    let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);

    let tasks = plan::plan(
        cfg,
        &Subcommand::TestAll,
        &mut interner,
        &info,
        &mut cache_state,
        0,
        ts,
    );

    assert!(tasks.is_empty());
}

fn check_store_task(
    task: &Task,
    interner: &Interner,
    path: &str,
    cache_files_path: &str,
    store_hashes: &mut Vec<Hash>,
) {
    assert!(task.outputs.is_empty());
    assert_eq!(strs(interner, &task.inputs), vec![path]);

    match &task.command {
        Some(Command::CacheStore(cs)) => {
            assert_eq!(cs.cache_files_path, cache_files_path);
            assert_eq!(cs.file_path, path);
            assert_ne!(cs.deep_hash, 0);
            store_hashes.push(cs.deep_hash);
        }
        _ => unreachable!(),
    }
}

// Check a CompileToObject task for a lib/test/bin part.
fn check_obj_task(
    task: &Task,
    interner: &Interner,
    build_path: &str,
    src_path: &str,
    entry_name: &str,
    part_name: &str,
    folder_name: &str,
) {
    assert_eq!(interner.get(task.part_name), part_name);

    let root_pcm_path = format!("{}/mod/{}.pcm", build_path, entry_name);
    assert_eq!(strs(interner, &task.inputs), vec![root_pcm_path.as_str()]);

    let obj_path = format!(
        "{}/entry/{}/{}/{}.o",
        build_path, entry_name, folder_name, part_name
    );
    assert_eq!(strs(interner, &task.outputs), vec![obj_path.as_str()]);

    match &task.command {
        Some(Command::CompileToObject(co)) => {
            assert_eq!(co.cxx, "clang++");
            assert_eq!(
                co.input,
                format!(
                    "{}/{}/{}/{}.cc",
                    src_path, entry_name, folder_name, part_name
                ),
            );
            assert_eq!(co.output, obj_path);
        }
        _ => unreachable!(),
    }
}

// Check a LinkBinary task for a test/bin part.
fn check_link_task(
    task: &Task,
    interner: &Interner,
    build_path: &str,
    entry_name: &str,
    part_name: &str,
    folder_name: &str,
    lib_paths: &[String],
) {
    assert_eq!(interner.get(task.part_name), part_name);

    let obj_path = format!(
        "{}/entry/{}/{}/{}.o",
        build_path, entry_name, folder_name, part_name
    );
    let exe_path = format!(
        "{}/entry/{}/{}/{}",
        build_path, entry_name, folder_name, part_name
    );

    let mut expected_inputs = as_strs(lib_paths);
    expected_inputs.push(obj_path.as_str());
    assert_eq!(strs(interner, &task.inputs), expected_inputs);

    assert_eq!(strs(interner, &task.outputs), vec![exe_path.as_str()]);

    match &task.command {
        Some(Command::LinkBinary(lb)) => {
            assert_eq!(lb.cxx, "clang++");
            assert_eq!(lb.output, exe_path);
            assert_eq!(lb.object, obj_path);
            assert_eq!(lb.deps.to_vec(), as_strs(lib_paths));
        }
        _ => unreachable!(),
    }
}

fn check_run_test_task(
    task: &Task,
    interner: &Interner,
    build_path: &str,
    entry_name: &str,
    test_name: &str,
) {
    assert_eq!(interner.get(task.part_name), test_name);

    let exe_path = format!("{}/entry/{}/test/{}", build_path, entry_name, test_name);
    let stamp_path = format!(
        "{}/entry/{}/test/stamp/{}",
        build_path, entry_name, test_name
    );

    assert_eq!(strs(interner, &task.inputs), vec![exe_path.as_str()]);
    assert_eq!(strs(interner, &task.outputs), vec![stamp_path.as_str()]);

    match &task.command {
        Some(Command::RunTest(rt)) => {
            assert_eq!(rt.path, exe_path);
            assert_eq!(rt.stamp_path, stamp_path);
            assert_ne!(rt.stamp_hash, 0);
        }
        _ => unreachable!(),
    }
}

// Check all tasks of one entry, in order. The expected tasks are derived from
// the scanned BuildInfo itself because the order of the parts comes from
// reading the source directories, which is not deterministic.
fn check_entry_tasks(
    entry_tasks: &[&Task],
    interner: &Interner,
    info: &BuildInfo,
    eidx: usize,
    build_path: &str,
    src_path: &str,
    lib_paths: &[String],
    cache_files_path: &str,
    store_hashes: &mut Vec<Hash>,
) {
    let entry_name = interner.get(info.name[eidx]).to_owned();
    let n_mod_parts = info.module[eidx].len();

    let mut i = 0;

    // Module tasks: for each module part a compile task followed by cache
    // store tasks for the pcm and the object outputs.
    for part_idx in 0..n_mod_parts {
        let part_name = interner.get(info.module[eidx][part_idx]).to_owned();

        let pcm_path = format!(
            "{}/mod/{}",
            build_path,
            make_module_artifact_name(&entry_name, &part_name, "pcm"),
        );
        let obj_path = format!(
            "{}/mod/{}",
            build_path,
            make_module_artifact_name(&entry_name, &part_name, "o"),
        );

        let mut expected_inputs: Vec<String> = Vec::new();
        for &dep in info.deps[eidx].iter() {
            expected_inputs.push(format!("{}/mod/{}.pcm", build_path, interner.get(dep)));
        }
        for dep_idx in 0..n_mod_parts {
            if info.mod_dep_matrix[eidx][part_idx * n_mod_parts + dep_idx] {
                let dep_name = interner.get(info.module[eidx][dep_idx]);
                expected_inputs.push(format!(
                    "{}/mod/{}",
                    build_path,
                    make_module_artifact_name(&entry_name, dep_name, "pcm"),
                ));
            }
        }

        {
            let task = entry_tasks[i];
            assert_eq!(interner.get(task.part_name), part_name);

            assert_eq!(strs(interner, &task.inputs), as_strs(&expected_inputs));
            assert_eq!(
                strs(interner, &task.outputs),
                vec![pcm_path.as_str(), obj_path.as_str()],
            );

            match &task.command {
                Some(Command::CompileModule(cm)) => {
                    assert_eq!(cm.cxx, "clang++");
                    assert_eq!(
                        cm.input,
                        format!("{}/{}/mod/{}.cppm", src_path, entry_name, part_name),
                    );
                    assert_eq!(cm.object_output, obj_path);
                    assert_eq!(cm.pcm_output, pcm_path);
                }
                _ => unreachable!(),
            }
        }
        i += 1;

        check_store_task(
            entry_tasks[i],
            interner,
            &pcm_path,
            cache_files_path,
            store_hashes,
        );
        i += 1;

        check_store_task(
            entry_tasks[i],
            interner,
            &obj_path,
            cache_files_path,
            store_hashes,
        );
        i += 1;
    }

    // Lib object tasks.
    for &part in info.lib[eidx].iter() {
        let part_name = interner.get(part);

        check_obj_task(
            entry_tasks[i],
            interner,
            build_path,
            src_path,
            &entry_name,
            part_name,
            "lib",
        );
        i += 1;

        let obj_path = format!("{}/entry/{}/lib/{}.o", build_path, entry_name, part_name);
        check_store_task(
            entry_tasks[i],
            interner,
            &obj_path,
            cache_files_path,
            store_hashes,
        );
        i += 1;
    }

    // Lib archive task. Inputs are all the module objects followed by all the
    // lib objects.
    {
        let mut expected_inputs: Vec<String> = Vec::new();
        for &part in info.module[eidx].iter() {
            let part_name = interner.get(part);
            expected_inputs.push(format!(
                "{}/mod/{}",
                build_path,
                make_module_artifact_name(&entry_name, part_name, "o"),
            ));
        }
        for &part in info.lib[eidx].iter() {
            let part_name = interner.get(part);
            expected_inputs.push(format!(
                "{}/entry/{}/lib/{}.o",
                build_path, entry_name, part_name
            ));
        }

        let lib_path = format!(
            "{}/entry/{}/lib/lib{}.a",
            build_path, entry_name, entry_name
        );

        let task = entry_tasks[i];
        assert_eq!(interner.get(task.part_name), "@ARCHIVE_LIB@");

        assert_eq!(strs(interner, &task.inputs), as_strs(&expected_inputs));
        assert_eq!(strs(interner, &task.outputs), vec![lib_path.as_str()]);

        match &task.command {
            Some(Command::ArchiveLib(al)) => {
                assert_eq!(al.ar, "llvm-ar");
                assert_eq!(al.output, lib_path);
                assert_eq!(al.inputs, as_strs(&expected_inputs));
            }
            _ => unreachable!(),
        }
    }
    i += 1;

    {
        let lib_path = format!(
            "{}/entry/{}/lib/lib{}.a",
            build_path, entry_name, entry_name
        );
        check_store_task(
            entry_tasks[i],
            interner,
            &lib_path,
            cache_files_path,
            store_hashes,
        );
    }
    i += 1;

    // Test tasks: obj, link, run for each test.
    for &part in info.test[eidx].iter() {
        let test_name = interner.get(part);

        check_obj_task(
            entry_tasks[i],
            interner,
            build_path,
            src_path,
            &entry_name,
            test_name,
            "test",
        );
        i += 1;

        let obj_path = format!("{}/entry/{}/test/{}.o", build_path, entry_name, test_name);
        check_store_task(
            entry_tasks[i],
            interner,
            &obj_path,
            cache_files_path,
            store_hashes,
        );
        i += 1;

        check_link_task(
            entry_tasks[i],
            interner,
            build_path,
            &entry_name,
            test_name,
            "test",
            lib_paths,
        );
        i += 1;

        let exe_path = format!("{}/entry/{}/test/{}", build_path, entry_name, test_name);
        check_store_task(
            entry_tasks[i],
            interner,
            &exe_path,
            cache_files_path,
            store_hashes,
        );
        i += 1;

        check_run_test_task(entry_tasks[i], interner, build_path, &entry_name, test_name);
        i += 1;

        let stamp_path = format!(
            "{}/entry/{}/test/stamp/{}",
            build_path, entry_name, test_name
        );
        check_store_task(
            entry_tasks[i],
            interner,
            &stamp_path,
            cache_files_path,
            store_hashes,
        );
        i += 1;
    }

    // Bin tasks: obj, link for each bin.
    for &part in info.bin[eidx].iter() {
        let bin_name = interner.get(part);

        check_obj_task(
            entry_tasks[i],
            interner,
            build_path,
            src_path,
            &entry_name,
            bin_name,
            "bin",
        );
        i += 1;

        let obj_path = format!("{}/entry/{}/bin/{}.o", build_path, entry_name, bin_name);
        check_store_task(
            entry_tasks[i],
            interner,
            &obj_path,
            cache_files_path,
            store_hashes,
        );
        i += 1;

        check_link_task(
            entry_tasks[i],
            interner,
            build_path,
            &entry_name,
            bin_name,
            "bin",
            lib_paths,
        );
        i += 1;

        let exe_path = format!("{}/entry/{}/bin/{}", build_path, entry_name, bin_name);
        check_store_task(
            entry_tasks[i],
            interner,
            &exe_path,
            cache_files_path,
            store_hashes,
        );
        i += 1;
    }

    assert_eq!(i, entry_tasks.len());
}

// A comparable representation of a task list, used to check that planning
// twice on the same input gives the same result.
fn task_debug_strings<'a>(
    tasks: impl IntoIterator<Item = &'a Task>,
    interner: &Interner,
) -> Vec<String> {
    tasks
        .into_iter()
        .map(|t| {
            format!(
                "{} {} {:?} {:?} {}",
                t.entry_index,
                interner.get(t.part_name),
                strs(interner, &t.inputs),
                strs(interner, &t.outputs),
                t.command.as_ref().unwrap().print(),
            )
        })
        .collect()
}

#[test]
fn test_simple() {
    let folder = TempDir::new();
    let cfg = make_config(folder.as_str());

    let src_path = std::path::Path::new(cfg.src_path);
    make_baselib(src_path);
    make_iolib(src_path);
    make_myapp(src_path);

    let thread_pool = ThreadPool::new(8);

    let ts = timestamp();
    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

    let mut interner = Interner::new();

    let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);

    let tasks = plan::plan(
        cfg,
        &Subcommand::TestAll,
        &mut interner,
        &info,
        &mut cache_state,
        0,
        ts,
    );

    assert_eq!(tasks.len(), NUM_TASKS);

    let build_path = cfg.build_path.to_str().unwrap();
    let src_path = cfg.src_path.to_str().unwrap();
    let cache_files_path = cfg.cache_path.to_str().unwrap();

    // Planning is deterministic: a second run with a fresh cache state must
    // produce the exact same tasks, including the deep hashes.
    {
        let mut cache_state = cache::load(
            Path::new(&format!("{}/cache2", folder.as_str())),
            &thread_pool,
            ts,
        );
        let tasks2 = plan::plan(
            cfg,
            &Subcommand::TestAll,
            &mut interner,
            &info,
            &mut cache_state,
            0,
            ts,
        );
        assert_eq!(
            task_debug_strings(&tasks, &interner),
            task_debug_strings(&tasks2, &interner),
        );
    }

    // Check every task of every entry in detail.
    let mut store_hashes: Vec<Hash> = Vec::new();

    for eidx in 0..info.name.len() {
        let entry_name = interner.get(info.name[eidx]);

        let entry_tasks: Vec<&Task> = tasks.iter().filter(|t| t.entry_index == eidx).collect();

        let expected_num_tasks = match entry_name {
            "base" => 43,
            "io" => 39,
            "myapp" => 9,
            _ => unreachable!(),
        };
        assert_eq!(entry_tasks.len(), expected_num_tasks);

        for task in entry_tasks.iter() {
            assert_eq!(interner.get(task.entry_name), entry_name);
        }

        // The library list used for linking. An entry always comes before the
        // entries it depends on so the linker sees them in the right order.
        let lib_paths: Vec<String> = match entry_name {
            "base" => vec![format!("{}/entry/base/lib/libbase.a", build_path)],
            "io" => vec![
                format!("{}/entry/io/lib/libio.a", build_path),
                format!("{}/entry/base/lib/libbase.a", build_path),
            ],
            "myapp" => vec![
                format!("{}/entry/myapp/lib/libmyapp.a", build_path),
                format!("{}/entry/io/lib/libio.a", build_path),
                format!("{}/entry/base/lib/libbase.a", build_path),
            ],
            _ => unreachable!(),
        };

        check_entry_tasks(
            &entry_tasks,
            &interner,
            &info,
            eidx,
            build_path,
            src_path,
            &lib_paths,
            cache_files_path,
            &mut store_hashes,
        );
    }

    // Every artifact that is stored to the cache has its own distinct deep
    // hash.
    assert_eq!(store_hashes.len(), NUM_STORED_ARTIFACTS);
    let mut seen = std::collections::HashSet::new();
    for hash in store_hashes.iter() {
        assert!(seen.insert(*hash));
    }

    // The `test` subcommand only runs the selected test.
    {
        let mut cache_state = cache::load(
            Path::new(&format!("{}/cache3", folder.as_str())),
            &thread_pool,
            ts,
        );
        let tasks = plan::plan(
            cfg,
            &Subcommand::Test {
                entry_name: "base".to_owned(),
                test_name: "memcmp".to_owned(),
            },
            &mut interner,
            &info,
            &mut cache_state,
            0,
            ts,
        );

        let run_tests: Vec<&Task> = tasks
            .iter()
            .filter(|t| matches!(t.command, Some(Command::RunTest(_))))
            .collect();
        assert_eq!(run_tests.len(), 1);
        assert_eq!(interner.get(run_tests[0].entry_name), "base");
        assert_eq!(interner.get(run_tests[0].part_name), "memcmp");

        assert!(
            tasks
                .iter()
                .all(|t| !matches!(t.command, Some(Command::RunBinary(_)))),
        );
    }

    // The `run` subcommand only runs the selected binary.
    {
        let mut cache_state = cache::load(
            Path::new(&format!("{}/cache4", folder.as_str())),
            &thread_pool,
            ts,
        );
        let tasks = plan::plan(
            cfg,
            &Subcommand::RunBinary {
                entry_name: "base".to_owned(),
                bin_name: "memcpy_bench".to_owned(),
                args: &["--some-arg"],
            },
            &mut interner,
            &info,
            &mut cache_state,
            0,
            ts,
        );

        let run_bins: Vec<&Task> = tasks
            .iter()
            .filter(|t| matches!(t.command, Some(Command::RunBinary(_))))
            .collect();
        assert_eq!(run_bins.len(), 1);
        assert_eq!(interner.get(run_bins[0].entry_name), "base");
        assert_eq!(interner.get(run_bins[0].part_name), "memcpy_bench");
        match &run_bins[0].command {
            Some(Command::RunBinary(rb)) => {
                assert_eq!(
                    rb.path,
                    format!("{}/entry/base/bin/memcpy_bench", build_path),
                );
                assert_eq!(rb.args.to_vec(), vec!["--some-arg"]);
            }
            _ => unreachable!(),
        }

        assert!(
            tasks
                .iter()
                .all(|t| !matches!(t.command, Some(Command::RunTest(_)))),
        );
    }

    // The `build` subcommand builds everything but doesn't run any test or
    // binary.
    {
        let mut cache_state = cache::load(
            Path::new(&format!("{}/cache5", folder.as_str())),
            &thread_pool,
            ts,
        );
        let tasks = plan::plan(
            cfg,
            &Subcommand::Build,
            &mut interner,
            &info,
            &mut cache_state,
            0,
            ts,
        );

        // Same as TestAll but without the run+store task pair of each of the
        // 6 tests.
        assert_eq!(tasks.len(), NUM_TASKS - 6 * 2);

        assert!(tasks.iter().all(|t| {
            !matches!(t.command, Some(Command::RunTest(_)))
                && !matches!(t.command, Some(Command::RunBinary(_)))
        }),);
    }

    // The `fmt` subcommand emits a format task for every source file and
    // nothing else.
    {
        let tasks = plan::plan(
            cfg,
            &Subcommand::Fmt,
            &mut interner,
            &info,
            &mut cache_state,
            0,
            ts,
        );

        assert_eq!(tasks.len(), 22);

        let mut formatted_paths: Vec<&'static str> = tasks
            .iter()
            .map(|t| match &t.command {
                Some(Command::Format(f)) => {
                    assert_eq!(f.clang_format, "clang-format");
                    assert!(t.inputs.is_empty());
                    assert!(t.outputs.is_empty());
                    f.path
                }
                _ => unreachable!(),
            })
            .collect();
        formatted_paths.sort_unstable();

        let mut expected_paths: Vec<&'static str> = Vec::new();
        for eidx in 0..info.name.len() {
            let entry_name = interner.get(info.name[eidx]);
            for (folder, ext) in [
                ("mod", "cppm"),
                ("lib", "cc"),
                ("bin", "cc"),
                ("test", "cc"),
            ] {
                let parts: Vec<&'static str> = match folder {
                    "mod" => info.module[eidx].iter().map(|&p| interner.get(p)).collect(),
                    "lib" => info.lib[eidx].iter().map(|&p| interner.get(p)).collect(),
                    "bin" => info.bin[eidx].iter().map(|&p| interner.get(p)).collect(),
                    "test" => info.test[eidx].iter().map(|&p| interner.get(p)).collect(),
                    _ => unreachable!(),
                };
                for part in parts {
                    let path = interner.insert(&format!(
                        "{}/{}/{}/{}.{}",
                        cfg.src_path.to_str().unwrap(),
                        entry_name,
                        folder,
                        part,
                        ext,
                    ));
                    expected_paths.push(interner.get(path));
                }
            }
        }
        expected_paths.sort_unstable();

        assert_eq!(formatted_paths, expected_paths);
    }
}

// Check the complete task list of a tree: the totals against the expected
// numbers and every entry's tasks in detail with `check_entry_tasks`.
//
// `expected_entries` has one (entry name, expected number of tasks, entry
// names of the libraries in link order) tuple per entry of the tree.
fn check_tree(
    folder: &str,
    make_tree: fn(&Path),
    expected_entries: &[(&str, usize, &[&str])],
    expected_total_tasks: usize,
    expected_total_stores: usize,
    thread_pool: &ThreadPool,
) {
    let cfg = make_config(folder);
    make_tree(cfg.src_path);

    let ts = timestamp();
    let mut cache_state = cache::load(cfg.cache_path, thread_pool, ts);

    let mut interner = Interner::new();

    let info = scan::scan(cfg, &mut interner, thread_pool, 0, &mut cache_state, ts);

    let tasks = plan::plan(
        cfg,
        &Subcommand::TestAll,
        &mut interner,
        &info,
        &mut cache_state,
        0,
        ts,
    );

    assert_eq!(info.name.len(), expected_entries.len());
    assert_eq!(tasks.len(), expected_total_tasks);

    let build_path = cfg.build_path.to_str().unwrap();
    let src_path = cfg.src_path.to_str().unwrap();
    let cache_files_path = cfg.cache_path.to_str().unwrap();

    let mut store_hashes: Vec<Hash> = Vec::new();

    for eidx in 0..info.name.len() {
        let entry_name = interner.get(info.name[eidx]);

        let (_, expected_num_tasks, lib_entry_names) = expected_entries
            .iter()
            .find(|(name, _, _)| *name == entry_name)
            .unwrap();

        let entry_tasks: Vec<&Task> = tasks.iter().filter(|t| t.entry_index == eidx).collect();
        assert_eq!(entry_tasks.len(), *expected_num_tasks);

        for task in entry_tasks.iter() {
            assert_eq!(interner.get(task.entry_name), entry_name);
        }

        // The library list used for linking: the entry itself first, then
        // the entries it depends on so the linker sees them in the right
        // order.
        let lib_paths: Vec<String> = lib_entry_names
            .iter()
            .map(|&e| format!("{}/entry/{}/lib/lib{}.a", build_path, e, e))
            .collect();

        check_entry_tasks(
            &entry_tasks,
            &interner,
            &info,
            eidx,
            build_path,
            src_path,
            &lib_paths,
            cache_files_path,
            &mut store_hashes,
        );
    }

    // Every artifact that is stored to the cache has its own distinct deep
    // hash.
    assert_eq!(store_hashes.len(), expected_total_stores);
    let mut seen = std::collections::HashSet::new();
    for hash in store_hashes.iter() {
        assert!(seen.insert(*hash));
    }
}

// Plan the additional trees and check every task they produce.
#[test]
fn test_more_trees() {
    let thread_pool = ThreadPool::new(8);

    // diamond: 4 modules * 3 + 1 lib obj * 2 + 1 archive * 2
    //          + 2 tests * 6 + 2 bins * 4               = 36 tasks,
    // 20 of which are cache stores.
    {
        let folder = TempDir::new();
        check_tree(
            folder.as_str(),
            make_diamond,
            &[("diamond", 36, &["diamond"])],
            36,
            20,
            &thread_pool,
        );
    }

    // util: 1 module * 3 + 1 lib obj * 2 + 1 archive * 2 + 1 test * 6
    //       + 1 bin * 4                                 = 17 tasks.
    // app:  2 modules * 3 + 0 lib objs * 2 + 1 archive * 2 + 1 test * 6
    //       + 0 bins * 4                                = 14 tasks.
    // 17 of the 31 tasks are cache stores.
    {
        let folder = TempDir::new();
        check_tree(
            folder.as_str(),
            make_util_app,
            &[("util", 17, &["util"]), ("app", 14, &["app", "util"])],
            31,
            17,
            &thread_pool,
        );
    }

    // core: 1 module * 3 + 1 lib obj * 2 + 1 archive * 2 + 1 test * 6
    //       + 0 bins * 4                                = 13 tasks.
    // mid:  1 module * 3 + 0 lib objs * 2 + 1 archive * 2 + 1 test * 6
    //       + 1 bin * 4                                 = 15 tasks.
    // top:  2 modules * 3 + 0 lib objs * 2 + 1 archive * 2 + 1 test * 6
    //       + 1 bin * 4                                 = 18 tasks.
    // 25 of the 46 tasks are cache stores.
    {
        let folder = TempDir::new();
        check_tree(
            folder.as_str(),
            make_chain,
            &[
                ("core", 13, &["core"]),
                ("mid", 15, &["mid", "core"]),
                ("top", 18, &["top", "mid", "core"]),
            ],
            46,
            25,
            &thread_pool,
        );
    }
}
// Write a cached file and a wal entry for it, like the build system does when
// it stores an artifact. The content cached for the artifact is the artifact
// path itself so the expected hash is easy to compute.
fn store_in_cache(cache_dir: &Path, path: &str, deep_hash: Hash) {
    let content = path.as_bytes();
    let output_hash = hash_bytes(content);
    std::fs::write(cache_dir.join(format!("{:x}", deep_hash)), content).unwrap();
    cache::write_wal_file(
        cache_dir,
        deep_hash,
        output_hash,
        u32::try_from(content.len()).unwrap(),
    );
}

// Check that plan accounts for the cache state correctly: a cached artifact
// turns into a cache load task instead of a compile/store pair, an artifact
// that is only partially cached (a module pcm without its object) is fully
// recompiled, and everything that isn't cached is planned exactly like on a
// cold cache.
#[test]
fn test_cache_state() {
    let folder = TempDir::new();
    let cfg = make_config(folder.as_str());

    let src_path = std::path::Path::new(cfg.src_path);
    make_baselib(src_path);
    make_iolib(src_path);
    make_myapp(src_path);

    let thread_pool = ThreadPool::new(8);

    let ts = timestamp();
    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

    let mut interner = Interner::new();

    let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);

    let build_path = cfg.build_path.to_str().unwrap();
    let cache_files_path = cfg.cache_path.to_str().unwrap();

    // A plan on a cold cache emits compile + cache store tasks for
    // everything.
    let cold_tasks = plan::plan(
        cfg,
        &Subcommand::TestAll,
        &mut interner,
        &info,
        &mut cache_state,
        0,
        ts,
    );
    assert_eq!(cold_tasks.len(), NUM_TASKS);

    // Collect the deep hash of every artifact from the cache store commands
    // of the cold plan.
    let mut deep_to_path: HashMap<Hash, &'static str> = HashMap::default();
    let mut path_to_deep: HashMap<&'static str, Hash> = HashMap::default();
    for task in cold_tasks.iter() {
        if let Some(Command::CacheStore(cs)) = &task.command {
            assert!(deep_to_path.insert(cs.deep_hash, cs.file_path).is_none());
            assert!(path_to_deep.insert(cs.file_path, cs.deep_hash).is_none());
        }
    }
    assert_eq!(deep_to_path.len(), NUM_STORED_ARTIFACTS);

    // A cache load must carry the same dependency inputs as the non-cache
    // task it replaces. Map every cold-plan output to that task's inputs.
    let cold_inputs: HashMap<Str, &[Str]> = cold_tasks
        .iter()
        .flat_map(|task| {
            task.outputs
                .iter()
                .map(|&output| (output, task.inputs.as_slice()))
        })
        .collect();

    // Fully warm cache: every artifact is a cache hit. The plan must emit
    // exactly one cache load task per artifact and nothing else. In
    // particular no test is re-run because their stamps are cached too.
    {
        for (&deep, &path) in deep_to_path.iter() {
            store_in_cache(cfg.cache_path, path, deep);
        }

        let mut warm_state = cache::load(cfg.cache_path, &thread_pool, ts);
        let warm_tasks = plan::plan(
            cfg,
            &Subcommand::TestAll,
            &mut interner,
            &info,
            &mut warm_state,
            0,
            ts,
        );

        assert_eq!(warm_tasks.len(), NUM_STORED_ARTIFACTS);

        let mut loaded_paths: Vec<&'static str> = Vec::new();
        for task in warm_tasks.iter() {
            match &task.command {
                Some(Command::CacheLoad(cl)) => {
                    assert_eq!(cl.cache_files_path, cache_files_path);
                    assert_eq!(task.outputs.len(), 1);
                    assert_eq!(interner.get(task.outputs[0]), cl.output_path);

                    let expected_inputs = cold_inputs.get(&task.outputs[0]).unwrap_or_else(|| {
                        panic!("no cold-plan task produces `{}`", cl.output_path)
                    });
                    assert_eq!(
                        task.inputs.as_slice(),
                        *expected_inputs,
                        "cache load for `{}` must have the same inputs as its non-cache task",
                        cl.output_path,
                    );

                    // The cache load refers to the right artifact and
                    // expects the hash of the content we cached for it.
                    assert_eq!(*deep_to_path.get(&cl.deep_hash).unwrap(), cl.output_path);
                    assert_eq!(cl.expected_hash, hash_bytes(cl.output_path.as_bytes()));

                    loaded_paths.push(cl.output_path);
                }
                _ => panic!(
                    "expected only cache loads on a warm cache, got: {}",
                    task.command.as_ref().unwrap().print(),
                ),
            }
        }

        // Every artifact is loaded from the cache.
        let mut expected_paths: Vec<&'static str> = deep_to_path.values().copied().collect();
        loaded_paths.sort_unstable();
        expected_paths.sort_unstable();
        assert_eq!(loaded_paths, expected_paths);
    }

    // Partially warm cache: only some artifacts are hits.
    {
        let partial_cache_dir = format!("{}/cache6", folder.as_str());
        let partial_cache_dir = Path::new(&partial_cache_dir);
        // Create the cache directories first.
        drop(cache::load(partial_cache_dir, &thread_pool, ts));

        // The root module of `base` is fully cached (both its pcm and object)
        // so it turns into two cache loads.
        let base_pcm = format!("{}/mod/base.pcm", build_path);
        let base_obj = format!("{}/mod/base.o", build_path);

        // The `base:vec` module has only its pcm cached. A module is only
        // loaded from the cache when both its pcm and object are cached, so
        // this one must be fully recompiled and re-stored instead.
        let vec_pcm = format!("{}/mod/base-vec.pcm", build_path);

        // The `memcmp` lib object of `base` is cached.
        let memcmp_obj = format!("{}/entry/base/lib/memcmp.o", build_path);

        for path in [&base_pcm, &base_obj, &vec_pcm, &memcmp_obj] {
            store_in_cache(
                partial_cache_dir,
                path.as_str(),
                *path_to_deep.get(path.as_str()).unwrap(),
            );
        }

        let mut partial_state = cache::load(partial_cache_dir, &thread_pool, ts);
        let partial_tasks = plan::plan(
            cfg,
            &Subcommand::TestAll,
            &mut interner,
            &info,
            &mut partial_state,
            0,
            ts,
        );

        // base:base module part: 3 tasks (compile + 2 stores) become 2 cache
        // loads. memcmp lib object: 2 tasks (compile + store) become 1 cache
        // load. base:vec is fully recompiled so it keeps its 3 tasks.
        assert_eq!(partial_tasks.len(), NUM_TASKS - 2);

        // Exactly the three fully cached artifacts are cache loads.
        let loads: Vec<&Task> = partial_tasks
            .iter()
            .filter(|t| matches!(t.command, Some(Command::CacheLoad(_))))
            .collect();
        assert_eq!(loads.len(), 3);

        let mut loaded_paths: Vec<String> = Vec::new();
        for task in loads.iter() {
            match &task.command {
                Some(Command::CacheLoad(cl)) => {
                    assert_eq!(cl.cache_files_path, cache_files_path);
                    assert_eq!(task.outputs.len(), 1);
                    assert_eq!(interner.get(task.outputs[0]), cl.output_path);

                    let expected_inputs = cold_inputs.get(&task.outputs[0]).unwrap_or_else(|| {
                        panic!("no cold-plan task produces `{}`", cl.output_path)
                    });
                    assert_eq!(
                        task.inputs.as_slice(),
                        *expected_inputs,
                        "cache load for `{}` must have the same inputs as its non-cache task",
                        cl.output_path,
                    );

                    assert_eq!(cl.expected_hash, hash_bytes(cl.output_path.as_bytes()));
                    assert_eq!(*path_to_deep.get(cl.output_path).unwrap(), cl.deep_hash,);
                    loaded_paths.push(cl.output_path.to_owned());
                }
                _ => unreachable!(),
            }
        }
        loaded_paths.sort_unstable();
        let mut expected_paths = vec![base_obj.clone(), base_pcm.clone(), memcmp_obj.clone()];
        expected_paths.sort_unstable();
        assert_eq!(loaded_paths, expected_paths);

        // Everything that isn't cached is planned exactly like on the cold
        // cache, with the tasks replaced by the cache loads removed.
        let is_replaced = |task: &Task| match &task.command {
            Some(Command::CompileModule(cm)) => cm.pcm_output == base_pcm,
            Some(Command::CompileToObject(co)) => co.output == memcmp_obj,
            Some(Command::CacheStore(cs)) => {
                cs.file_path == base_pcm || cs.file_path == base_obj || cs.file_path == memcmp_obj
            }
            _ => false,
        };

        let cold_rest =
            task_debug_strings(cold_tasks.iter().filter(|t| !is_replaced(t)), &interner);
        let partial_rest = task_debug_strings(
            partial_tasks
                .iter()
                .filter(|t| !matches!(t.command, Some(Command::CacheLoad(_)))),
            &interner,
        );

        // The `base:vec` module is among the remaining tasks: its compile
        // task and both of its store tasks (including the pcm store) are
        // still there because it was only partially cached.
        assert_eq!(cold_rest, partial_rest);
    }
}

// A `.pcm` is not self-contained: clang loads and validates the PCMs it was
// built against. If a cached root module PCM is loaded before its
// submodule/dependency PCMs (which can happen after LRU pruning evicts only
// part of a module group), consumers fail with "module file not found" or
// "module file ... is out of date".
//
// This test creates a real cache with a cold build, removes the submodule PCMs
// and objects of `base`/`io` while keeping their root PCMs/objects and
// myapp's root PCM, and then replans and rebuilds. The cached PCM loads must
// depend on the PCMs they were built against, and the rebuild must succeed.
#[test]
fn test_partial_cache_pcm_dependencies() {
    let folder = TempDir::new();
    let cfg = make_config(folder.as_str());
    make_baselib(cfg.src_path);
    make_iolib(cfg.src_path);
    make_myapp(cfg.src_path);

    let thread_pool = ThreadPool::new(8);
    let mut interner = Interner::new();
    let build_path = cfg.build_path.to_str().unwrap();

    // Cold build so the cache contains real (valid) artifacts. Collect every
    // stored artifact's deep hash so selected entries can be removed below.
    let cold_artifacts: Vec<(Hash, &'static str)> = {
        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);
        let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);
        directory::setup_build_directories(cfg, &info, &interner, &thread_pool);
        let tasks = plan::plan(
            cfg,
            &Subcommand::Build,
            &mut interner,
            &info,
            &mut cache_state,
            0,
            ts,
        );
        let artifacts = tasks
            .iter()
            .filter_map(|task| match &task.command {
                Some(Command::CacheStore(cs)) => Some((cs.deep_hash, cs.file_path)),
                _ => None,
            })
            .collect();
        Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();
        cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);
        artifacts
    };

    // Keep the root PCMs and objects of base/io, remove their submodule PCMs
    // and objects, and force myapp's bin object to be recompiled. This is the
    // kind of partial state that pruning individual cache entries can leave
    // behind.
    let base_pcm = format!("{}/mod/base.pcm", build_path);
    let myapp_bin_obj = format!("{}/entry/myapp/bin/myapp.o", build_path);

    let state_file = cfg.cache_path.join("cache_state");
    let state_bytes = std::fs::read(&state_file).unwrap();
    let mut entries = cache::try_deserialize_state_file(&state_bytes).unwrap();

    let mut removed_submodule_pcm = false;
    let mut removed_myapp_bin_obj = false;

    for (deep_hash, path) in cold_artifacts.iter() {
        let is_submodule = path.contains("/mod/base-") || path.contains("/mod/io-");
        let is_myapp_bin_obj = *path == myapp_bin_obj.as_str();
        if !is_submodule && !is_myapp_bin_obj {
            continue;
        }

        assert!(entries.remove(deep_hash).is_some());
        std::fs::remove_file(cfg.cache_path.join(format!("{:x}", *deep_hash))).unwrap();

        removed_submodule_pcm |= is_submodule && path.ends_with(".pcm");
        removed_myapp_bin_obj |= is_myapp_bin_obj;
    }

    assert!(removed_submodule_pcm, "no submodule PCMs were removed");
    assert!(removed_myapp_bin_obj, "myapp's bin object was not removed");

    let mut new_state = Vec::new();
    cache::serialize_state_file(&entries, &mut new_state);
    let tmp = format!("{}.tmp", state_file.display());
    std::fs::write(&tmp, &new_state).unwrap();
    std::fs::rename(&tmp, &state_file).unwrap();

    // Replan. `base.pcm` is a cache hit while its submodule PCMs are misses,
    // and myapp's bin object is a miss that imports base and io.
    let ts = timestamp();
    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);
    let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);
    directory::setup_build_directories(cfg, &info, &interner, &thread_pool);
    let tasks = plan::plan(
        cfg,
        &Subcommand::Build,
        &mut interner,
        &info,
        &mut cache_state,
        0,
        ts,
    );

    // This is the regression assertion. Before the fix, the cached base.pcm
    // load had no inputs, so its submodule PCMs could still be compiling when
    // a consumer (myapp's bin object, or another cached PCM) was compiled.
    let base_pcm_load = tasks
        .iter()
        .find(|task| {
            matches!(
                &task.command,
                Some(Command::CacheLoad(cl)) if cl.output_path == base_pcm.as_str()
            )
        })
        .expect("expected base.pcm to be a cache load");
    let dependency_paths: Vec<&str> = base_pcm_load
        .inputs
        .iter()
        .map(|&idx| interner.get(idx))
        .collect();

    for part in ["vec", "start", "arith", "alloc"] {
        let expected = format!("{}/mod/base-{}.pcm", build_path, part);
        assert!(
            dependency_paths.contains(&expected.as_str()),
            "cached base.pcm must wait for `{expected}`; got {dependency_paths:?}"
        );
    }

    // Before the fix the assertion above fails first, so this build is only
    // reached once cached PCMs have their dependencies. After the fix it also
    // verifies the real compiler accepts the resulting schedule.
    Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();
    cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);

    assert!(cfg.build_path.join("mod/base.pcm").exists());
    assert!(cfg.build_path.join("entry/myapp/bin/myapp.o").exists());
}

// The object file itself is self-contained at link time, but the compile that
// regenerates it is not: it needs the PCMs it imports. This test keeps every
// PCM cached and only removes the lib/bin/test object files. That forces the
// object compilations to run again, which exposes the same missing PCM
// dependency edge as `test_partial_cache_pcm_dependencies`: a cached PCM that
// is an input to the object compile is not ordered before that compile unless
// the PCM load itself depends on its PCMs.
#[test]
fn test_partial_cache_object_dependencies() {
    let folder = TempDir::new();
    let cfg = make_config(folder.as_str());
    make_baselib(cfg.src_path);
    make_iolib(cfg.src_path);
    make_myapp(cfg.src_path);

    let thread_pool = ThreadPool::new(8);
    let mut interner = Interner::new();
    let build_path = cfg.build_path.to_str().unwrap();

    // Cold build with real artifacts, then collect every stored artifact.
    let cold_artifacts: Vec<(Hash, &'static str)> = {
        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);
        let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);
        directory::setup_build_directories(cfg, &info, &interner, &thread_pool);
        let tasks = plan::plan(
            cfg,
            &Subcommand::Build,
            &mut interner,
            &info,
            &mut cache_state,
            0,
            ts,
        );
        let artifacts = tasks
            .iter()
            .filter_map(|task| match &task.command {
                Some(Command::CacheStore(cs)) => Some((cs.deep_hash, cs.file_path)),
                _ => None,
            })
            .collect();
        Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();
        cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);
        artifacts
    };

    // Keep all PCMs, module objects, archives, executables and stamps. Remove
    // only the lib/bin/test object files.
    let myapp_bin_obj = format!("{}/entry/myapp/bin/myapp.o", build_path);
    let base_pcm = format!("{}/mod/base.pcm", build_path);
    let io_pcm = format!("{}/mod/io.pcm", build_path);

    let state_file = cfg.cache_path.join("cache_state");
    let state_bytes = std::fs::read(&state_file).unwrap();
    let mut entries = cache::try_deserialize_state_file(&state_bytes).unwrap();

    let mut removed_objects = 0;
    for (deep_hash, path) in cold_artifacts.iter() {
        if !(path.contains("/entry/") && path.ends_with(".o")) {
            continue;
        }

        assert!(entries.remove(deep_hash).is_some());
        std::fs::remove_file(cfg.cache_path.join(format!("{:x}", *deep_hash))).unwrap();
        removed_objects += 1;
    }
    assert!(removed_objects > 0, "no object files were removed");

    let mut new_state = Vec::new();
    cache::serialize_state_file(&entries, &mut new_state);
    let tmp = format!("{}.tmp", state_file.display());
    std::fs::write(&tmp, &new_state).unwrap();
    std::fs::rename(&tmp, &state_file).unwrap();

    // Replan and rebuild. All PCMs are cache hits, but myapp's bin object is
    // recompiled and imports both base and io.
    let ts = timestamp();
    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);
    let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);
    directory::setup_build_directories(cfg, &info, &interner, &thread_pool);
    let tasks = plan::plan(
        cfg,
        &Subcommand::Build,
        &mut interner,
        &info,
        &mut cache_state,
        0,
        ts,
    );

    let producers: HashMap<&str, &Task> = {
        let mut producers = HashMap::default();
        for task in tasks.iter() {
            for &output in task.outputs.iter() {
                producers.insert(interner.get(output), task);
            }
        }
        producers
    };

    let myapp_bin_obj_task = tasks
        .iter()
        .find(|task| {
            matches!(
                &task.command,
                Some(Command::CompileToObject(co)) if co.output == myapp_bin_obj.as_str()
            )
        })
        .expect("expected myapp's bin object to be recompiled");

    // Walk the task graph backwards from the object compile. The object's only
    // direct input is myapp.pcm, so base.pcm/io.pcm must be reached through the
    // cached myapp.pcm load. Before the fix that load had no inputs and the
    // dependency chain stops at myapp.pcm.
    let mut stack: Vec<&str> = myapp_bin_obj_task
        .inputs
        .iter()
        .map(|&idx| interner.get(idx))
        .collect();
    let mut visited: Vec<&str> = Vec::new();
    while let Some(path) = stack.pop() {
        if visited.contains(&path) {
            continue;
        }
        visited.push(path);

        if let Some(producer) = producers.get(path) {
            stack.extend(producer.inputs.iter().map(|&idx| interner.get(idx)));
        }
    }

    for expected in [base_pcm.as_str(), io_pcm.as_str()] {
        assert!(
            visited.contains(&expected),
            "myapp's bin object compile must be ordered after `{expected}`; \
             reached only {visited:?}"
        );
    }

    // As in the PCM test, the assertion above fails first before the fix, so
    // this build is only reached once object compiles are ordered after their
    // PCM dependencies.
    Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();
    cache::finish(cache_state, cfg.cache_path, &thread_pool, ts);

    assert!(cfg.build_path.join("mod/base.pcm").exists());
    assert!(cfg.build_path.join("entry/myapp/bin/myapp.o").exists());
}

fn find_entry(info: &BuildInfo, interner: &mut Interner, name: &str) -> usize {
    usize::try_from(*info.map.get(&interner.insert(name)).unwrap()).unwrap()
}

// The deep hashes are what decide whether an artifact can be loaded from the
// cache. They must change when anything the artifact depends on changes, and
// they must not change when unrelated things change. Getting this wrong means
// either stale binaries or useless cache misses.
#[test]
fn test_deep_hash_invalidation() {
    let folder = TempDir::new();
    let cfg = make_config(folder.as_str());
    make_chain(cfg.src_path);

    let thread_pool = ThreadPool::new(4);
    let ts = timestamp();
    let mut interner = Interner::new();

    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);
    let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);
    let before = plan::construct_deep_hashes(cfg, &mut interner, &info, 0);

    // Change a source file that only ends up in `core`'s library archive. The
    // module interface of `core` doesn't change, so module artifacts and the
    // library archives of the dependent entries must stay the same, while the
    // archive of `core` and everything linked from it must change.
    std::fs::write(
        cfg.src_path.join("core/lib/base_utils.cc"),
        "import core;\n// changed\n",
    )
    .unwrap();

    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);
    let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);
    let after = plan::construct_deep_hashes(cfg, &mut interner, &info, 0);

    let core = find_entry(&info, &mut interner, "core");
    let mid = find_entry(&info, &mut interner, "mid");
    let top = find_entry(&info, &mut interner, "top");

    assert_eq!(before.entry_pcm[core], after.entry_pcm[core]);
    assert_eq!(before.module_pcm[core], after.module_pcm[core]);
    assert_eq!(before.module_object[core], after.module_object[core]);
    assert_eq!(before.test_object[core], after.test_object[core]);

    assert_ne!(before.lib_object[core], after.lib_object[core]);
    assert_ne!(
        before.entry_lib_archive[core],
        after.entry_lib_archive[core]
    );
    assert_ne!(before.test_exe[core], after.test_exe[core]);

    // Dependents link `core`'s library but don't recompile their own modules
    // or libraries.
    assert_eq!(before.entry_pcm[mid], after.entry_pcm[mid]);
    assert_eq!(before.entry_lib_archive[mid], after.entry_lib_archive[mid]);
    assert_ne!(before.bin_exe[mid], after.bin_exe[mid]);
    assert_ne!(before.test_exe[mid], after.test_exe[mid]);

    assert_eq!(before.entry_pcm[top], after.entry_pcm[top]);
    assert_eq!(before.entry_lib_archive[top], after.entry_lib_archive[top]);
    assert_ne!(before.bin_exe[top], after.bin_exe[top]);
    assert_ne!(before.test_exe[top], after.test_exe[top]);

    // Now change the module interface of `core`. This propagates through every
    // entry that (transitively) imports it.
    let before = after;
    std::fs::write(
        cfg.src_path.join("core/mod/core.cppm"),
        "export module core;\n// changed\n",
    )
    .unwrap();

    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);
    let info = scan::scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);
    let after = plan::construct_deep_hashes(cfg, &mut interner, &info, 0);

    let core = find_entry(&info, &mut interner, "core");
    let mid = find_entry(&info, &mut interner, "mid");
    let top = find_entry(&info, &mut interner, "top");

    assert_ne!(before.module_pcm[core], after.module_pcm[core]);
    assert_ne!(before.entry_pcm[core], after.entry_pcm[core]);
    assert_ne!(before.lib_object[core], after.lib_object[core]);
    assert_ne!(
        before.entry_lib_archive[core],
        after.entry_lib_archive[core]
    );
    assert_ne!(before.test_exe[core], after.test_exe[core]);

    assert_ne!(before.entry_pcm[mid], after.entry_pcm[mid]);
    assert_ne!(before.entry_lib_archive[mid], after.entry_lib_archive[mid]);
    assert_ne!(before.bin_exe[mid], after.bin_exe[mid]);
    assert_ne!(before.test_exe[mid], after.test_exe[mid]);

    assert_ne!(before.entry_pcm[top], after.entry_pcm[top]);
    assert_ne!(before.entry_lib_archive[top], after.entry_lib_archive[top]);
    assert_ne!(before.bin_exe[top], after.bin_exe[top]);
    assert_ne!(before.test_exe[top], after.test_exe[top]);
}
