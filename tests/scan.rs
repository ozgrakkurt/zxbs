#![allow(clippy::single_element_loop)]

use zxbs::cache;
use zxbs::config::{Config, Subcommand};
use zxbs::hash::{HashMap, hash_ordered};
use zxbs::interner::{Interner, Str};
use zxbs::scan::{
    BuildInfo, check_cyclic_dependencies_between_entries,
    check_cyclic_dependencies_between_module_parts, scan,
};
use zxbs::thread_pool::ThreadPool;
use zxbs::timestamp::timestamp;

mod make_folders;
use make_folders::{make_baselib, make_chain, make_diamond, make_iolib, make_myapp, make_util_app};

mod tmp_dir;
use tmp_dir::TempDir;

#[test]
fn test_empty() {
    let folder = TempDir::new();
    let build_path = format!("{}/build", folder.as_str());
    let src_path = format!("{}/src", folder.as_str());
    std::fs::create_dir_all(&src_path).unwrap();

    let mut flags = HashMap::default();
    flags.insert("--build-dir".to_owned(), Some(build_path.clone()));
    flags.insert("--src-dir".to_owned(), Some(src_path.clone()));
    let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

    let thread_pool = ThreadPool::new(4);

    let ts = timestamp();
    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

    let mut interner = Interner::new();

    let info = scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);

    assert!(info.map.is_empty());
    assert!(info.name.is_empty());
    assert!(info.module.is_empty());
    assert!(info.mod_dep_matrix.is_empty());
    assert!(info.lib.is_empty());
    assert!(info.bin.is_empty());
    assert!(info.test.is_empty());
    assert!(info.deps.is_empty());
    assert!(info.basic_hash.is_empty());
}

fn check_baselib_info(info: &BuildInfo, interner: &mut Interner) {
    let name_idx = interner.insert("base");
    let i = info.name.iter().position(|&n| n == name_idx).unwrap();

    assert_eq!(
        usize::try_from(*info.map.get(&name_idx).unwrap()).unwrap(),
        i,
    );

    let mut part_idx_map: HashMap<&'static str, usize> = HashMap::default();

    assert_eq!(info.module[i].len(), 5);
    for name in ["base", "vec", "start", "arith", "alloc"] {
        let n_i = interner.insert(name);
        let pos = info.module[i].iter().position(|x| *x == n_i).unwrap();
        part_idx_map.insert(name, pos);
    }

    let mut mod_dep_matrix = vec![false; 25];

    {
        let idx = *part_idx_map.get("base").unwrap();
        for name in ["vec", "start", "arith", "alloc"] {
            let dep_idx = *part_idx_map.get(name).unwrap();
            mod_dep_matrix[idx * 5 + dep_idx] = true;
        }
    }

    {
        let idx = *part_idx_map.get("vec").unwrap();
        for name in ["arith", "alloc"] {
            let dep_idx = *part_idx_map.get(name).unwrap();
            mod_dep_matrix[idx * 5 + dep_idx] = true;
        }
    }

    {
        let idx = *part_idx_map.get("arith").unwrap();
        for name in ["start"] {
            let dep_idx = *part_idx_map.get(name).unwrap();
            mod_dep_matrix[idx * 5 + dep_idx] = true;
        }
    }

    {
        let idx = *part_idx_map.get("alloc").unwrap();
        for name in ["start", "arith"] {
            let dep_idx = *part_idx_map.get(name).unwrap();
            mod_dep_matrix[idx * 5 + dep_idx] = true;
        }
    }

    assert_eq!(&mod_dep_matrix, &info.mod_dep_matrix[i],);

    assert_eq!(info.lib[i].len(), 2);
    for name in ["memcmp", "memcpy"] {
        assert!(info.lib[i].contains(&interner.insert(name)));
    }

    assert_eq!(info.bin[i].len(), 1);
    for name in ["memcpy_bench"] {
        assert!(info.bin[i].contains(&interner.insert(name)));
    }

    assert_eq!(info.test[i].len(), 3);
    for name in ["memcmp", "vec", "arith"] {
        assert!(info.test[i].contains(&interner.insert(name)));
    }

    assert!(info.deps[i].is_empty());
}

fn check_iolib_info(info: &BuildInfo, interner: &mut Interner) {
    let name_idx = interner.insert("io");
    let i = info.name.iter().position(|&n| n == name_idx).unwrap();

    assert_eq!(
        usize::try_from(*info.map.get(&name_idx).unwrap()).unwrap(),
        i,
    );

    let mut part_idx_map: HashMap<&'static str, usize> = HashMap::default();

    assert_eq!(info.module[i].len(), 5);
    for name in ["io", "executor", "fs", "net", "base"] {
        let n_i = interner.insert(name);
        let pos = info.module[i].iter().position(|x| *x == n_i).unwrap();
        part_idx_map.insert(name, pos);
    }

    let mut mod_dep_matrix = vec![false; 25];

    {
        let idx = *part_idx_map.get("io").unwrap();
        for name in ["executor", "fs", "net", "base"] {
            let dep_idx = *part_idx_map.get(name).unwrap();
            mod_dep_matrix[idx * 5 + dep_idx] = true;
        }
    }

    {
        let idx = *part_idx_map.get("fs").unwrap();
        for name in ["executor"] {
            let dep_idx = *part_idx_map.get(name).unwrap();
            mod_dep_matrix[idx * 5 + dep_idx] = true;
        }
    }

    {
        let idx = *part_idx_map.get("net").unwrap();
        for name in ["executor"] {
            let dep_idx = *part_idx_map.get(name).unwrap();
            mod_dep_matrix[idx * 5 + dep_idx] = true;
        }
    }

    assert_eq!(&mod_dep_matrix, &info.mod_dep_matrix[i]);

    assert!(info.lib[i].is_empty());

    assert_eq!(info.bin[i].len(), 1);
    for name in ["direct_io_example"] {
        assert!(info.bin[i].contains(&interner.insert(name)));
    }

    assert_eq!(info.test[i].len(), 3);
    for name in ["executor_ordering", "fs", "net"] {
        assert!(info.test[i].contains(&interner.insert(name)));
    }

    assert_eq!(info.deps[i], &[interner.insert("base")]);
}

fn check_myapp_info(info: &BuildInfo, interner: &mut Interner) {
    let name_idx = interner.insert("myapp");
    let i = info.name.iter().position(|&n| n == name_idx).unwrap();

    assert_eq!(
        usize::try_from(*info.map.get(&name_idx).unwrap()).unwrap(),
        i,
    );

    let mut part_idx_map: HashMap<&'static str, usize> = HashMap::default();

    assert_eq!(info.module[i].len(), 1);
    for name in ["myapp"] {
        let n_i = interner.insert(name);
        let pos = info.module[i].iter().position(|x| *x == n_i).unwrap();
        part_idx_map.insert(name, pos);
    }

    let mod_dep_matrix = vec![false; 1];

    assert_eq!(&mod_dep_matrix, &info.mod_dep_matrix[i]);

    assert!(info.lib[i].is_empty());

    assert_eq!(info.bin[i].len(), 1);
    for name in ["myapp"] {
        assert!(info.bin[i].contains(&interner.insert(name)));
    }

    assert!(info.test[i].is_empty());

    for name in ["base", "io"] {
        assert!(info.deps[i].contains(&interner.insert(name)));
    }
}

// Find the index of an entry by name and check the name => index map agrees
// with it.
fn entry_index(info: &BuildInfo, interner: &mut Interner, name: &str) -> usize {
    let name_idx = interner.insert(name);
    let i = info.name.iter().position(|&n| n == name_idx).unwrap();
    assert_eq!(
        usize::try_from(*info.map.get(&name_idx).unwrap()).unwrap(),
        i,
    );
    i
}

// Check the module parts of an entry and its module dependency matrix. The
// expected matrix is given as (part, submodule dependency) pairs.
fn check_modules(
    info: &BuildInfo,
    interner: &mut Interner,
    eidx: usize,
    expected_parts: &[&'static str],
    expected_module_deps: &[(&'static str, &'static str)],
) {
    let n = expected_parts.len();
    assert_eq!(info.module[eidx].len(), n);

    let mut part_idx_map: HashMap<&'static str, usize> = HashMap::default();
    for &name in expected_parts {
        let n_i = interner.insert(name);
        let pos = info.module[eidx].iter().position(|&x| x == n_i).unwrap();
        part_idx_map.insert(name, pos);
    }

    let mut mod_dep_matrix = vec![false; n * n];

    for &(part, dep) in expected_module_deps {
        let idx = *part_idx_map.get(part).unwrap();
        let dep_idx = *part_idx_map.get(dep).unwrap();
        mod_dep_matrix[idx * n + dep_idx] = true;
    }

    assert_eq!(&mod_dep_matrix, &info.mod_dep_matrix[eidx]);
}

// Check the list of parts in one of the lib/bin/test folders of an entry.
fn check_part_list(list: &[Str], interner: &mut Interner, expected: &[&'static str]) {
    assert_eq!(list.len(), expected.len());
    for &name in expected {
        assert!(list.contains(&interner.insert(name)));
    }
}

// Check the entry dependencies of an entry.
fn check_deps(info: &BuildInfo, interner: &mut Interner, eidx: usize, expected: &[&'static str]) {
    assert_eq!(info.deps[eidx].len(), expected.len());
    for &name in expected {
        assert!(info.deps[eidx].contains(&interner.insert(name)));
    }
}

// Every source file of an entry must have a basic hash recorded for it and
// the hash must just be the hash of the path and the file contents.
fn check_basic_hash(info: &BuildInfo, interner: &Interner, path: String) {
    let mut found = 0;
    for (key, &hash) in info.basic_hash.iter() {
        if interner.get(*key) == path {
            let data = std::fs::read(&path).unwrap();
            assert_eq!(hash, hash_ordered(&[path.as_bytes(), data.as_slice()]));
            found += 1;
        }
    }
    assert_eq!(found, 1, "expected exactly one basic hash for `{}`", path);
}

fn check_all_basic_hashes(
    info: &BuildInfo,
    interner: &Interner,
    src_path: &str,
    expected_count: usize,
) {
    let mut num_checked = 0;

    for i in 0..info.name.len() {
        let entry_name = interner.get(info.name[i]);

        for folder_and_ext in [
            ("mod", "cppm"),
            ("lib", "cc"),
            ("bin", "cc"),
            ("test", "cc"),
        ] {
            let (folder, ext) = folder_and_ext;

            let parts: Vec<&str> = match folder {
                "mod" => info.module[i].iter().map(|&p| interner.get(p)).collect(),
                "lib" => info.lib[i].iter().map(|&p| interner.get(p)).collect(),
                "bin" => info.bin[i].iter().map(|&p| interner.get(p)).collect(),
                "test" => info.test[i].iter().map(|&p| interner.get(p)).collect(),
                _ => unreachable!(),
            };

            for part in parts {
                let path = format!("{}/{}/{}/{}.{}", src_path, entry_name, folder, part, ext);
                check_basic_hash(info, interner, path);
                num_checked += 1;
            }
        }
    }

    assert_eq!(num_checked, expected_count);
    assert_eq!(info.basic_hash.len(), expected_count);
}

#[test]
fn test_simple() {
    let folder = TempDir::new();
    let build_path = format!("{}/build", folder.as_str());
    let src_path = format!("{}/src", folder.as_str());
    std::fs::create_dir_all(&src_path).unwrap();

    // create folders/files
    make_baselib(std::path::Path::new(&src_path));
    make_iolib(std::path::Path::new(&src_path));
    make_myapp(std::path::Path::new(&src_path));

    let mut flags = HashMap::default();
    flags.insert("--build-dir".to_owned(), Some(build_path.clone()));
    flags.insert("--src-dir".to_owned(), Some(src_path.clone()));
    let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

    let thread_pool = ThreadPool::new(4);

    let ts = timestamp();
    let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

    let mut interner = Interner::new();

    let info = scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);

    assert_eq!(info.map.len(), 3);
    assert_eq!(info.name.len(), 3);
    assert_eq!(info.module.len(), 3);
    assert_eq!(info.mod_dep_matrix.len(), 3);
    assert_eq!(info.lib.len(), 3);
    assert_eq!(info.bin.len(), 3);
    assert_eq!(info.test.len(), 3);
    assert_eq!(info.deps.len(), 3);

    check_baselib_info(&info, &mut interner);
    check_iolib_info(&info, &mut interner);
    check_myapp_info(&info, &mut interner);

    check_all_basic_hashes(&info, &interner, cfg.src_path.to_str().unwrap(), 22);

    // check_subcommand accepts every subcommand that refers to parts that
    // actually exist.
    info.check_subcommand(
        &mut interner,
        &Subcommand::Test {
            entry_name: "base".to_owned(),
            test_name: "memcmp".to_owned(),
        },
    );
    info.check_subcommand(
        &mut interner,
        &Subcommand::RunBinary {
            entry_name: "base".to_owned(),
            bin_name: "memcpy_bench".to_owned(),
            args: &[],
        },
    );
    info.check_subcommand(&mut interner, &Subcommand::TestAll);
    info.check_subcommand(&mut interner, &Subcommand::Build);
}

// Scan the additional trees and check the build info derived from them.
#[test]
fn test_more_trees() {
    // A single entry with a diamond shaped module dependency graph, two bins
    // and two tests.
    {
        let folder = TempDir::new();
        let build_path = format!("{}/build", folder.as_str());
        let src_path = format!("{}/src", folder.as_str());
        std::fs::create_dir_all(&src_path).unwrap();

        make_diamond(std::path::Path::new(&src_path));

        let mut flags = HashMap::default();
        flags.insert("--build-dir".to_owned(), Some(build_path.clone()));
        flags.insert("--src-dir".to_owned(), Some(src_path.clone()));
        let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

        let thread_pool = ThreadPool::new(4);

        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

        let mut interner = Interner::new();

        let info = scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);

        assert_eq!(info.map.len(), 1);
        let i = entry_index(&info, &mut interner, "diamond");
        check_modules(
            &info,
            &mut interner,
            i,
            &["diamond", "left", "right", "common"],
            &[
                ("diamond", "left"),
                ("diamond", "right"),
                ("diamond", "common"),
                ("left", "common"),
                ("right", "common"),
            ],
        );
        check_part_list(&info.lib[i], &mut interner, &["helper"]);
        check_part_list(&info.bin[i], &mut interner, &["tool_a", "tool_b"]);
        check_part_list(&info.test[i], &mut interner, &["left", "right"]);
        check_deps(&info, &mut interner, i, &[]);
        check_all_basic_hashes(&info, &interner, cfg.src_path.to_str().unwrap(), 9);

        info.check_subcommand(
            &mut interner,
            &Subcommand::Test {
                entry_name: "diamond".to_owned(),
                test_name: "left".to_owned(),
            },
        );
        info.check_subcommand(
            &mut interner,
            &Subcommand::RunBinary {
                entry_name: "diamond".to_owned(),
                bin_name: "tool_b".to_owned(),
                args: &[],
            },
        );
    }

    // Two entries where `app` depends on `util` and imports its root module
    // from both the root module and a submodule. `app` has no lib or bin
    // parts.
    {
        let folder = TempDir::new();
        let build_path = format!("{}/build", folder.as_str());
        let src_path = format!("{}/src", folder.as_str());
        std::fs::create_dir_all(&src_path).unwrap();

        make_util_app(std::path::Path::new(&src_path));

        let mut flags = HashMap::default();
        flags.insert("--build-dir".to_owned(), Some(build_path.clone()));
        flags.insert("--src-dir".to_owned(), Some(src_path.clone()));
        let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

        let thread_pool = ThreadPool::new(4);

        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

        let mut interner = Interner::new();

        let info = scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);

        assert_eq!(info.map.len(), 2);

        let util = entry_index(&info, &mut interner, "util");
        check_modules(&info, &mut interner, util, &["util"], &[]);
        check_part_list(&info.lib[util], &mut interner, &["string"]);
        check_part_list(&info.bin[util], &mut interner, &["inspect"]);
        check_part_list(&info.test[util], &mut interner, &["string"]);
        check_deps(&info, &mut interner, util, &[]);

        let app = entry_index(&info, &mut interner, "app");
        check_modules(
            &info,
            &mut interner,
            app,
            &["app", "core"],
            &[("app", "core")],
        );
        check_part_list(&info.lib[app], &mut interner, &[]);
        check_part_list(&info.bin[app], &mut interner, &[]);
        check_part_list(&info.test[app], &mut interner, &["main"]);
        check_deps(&info, &mut interner, app, &["util"]);

        check_all_basic_hashes(&info, &interner, cfg.src_path.to_str().unwrap(), 7);
    }

    // A linear chain of three entries: `top` depends on `mid` which depends
    // on `core`.
    {
        let folder = TempDir::new();
        let build_path = format!("{}/build", folder.as_str());
        let src_path = format!("{}/src", folder.as_str());
        std::fs::create_dir_all(&src_path).unwrap();

        make_chain(std::path::Path::new(&src_path));

        let mut flags = HashMap::default();
        flags.insert("--build-dir".to_owned(), Some(build_path.clone()));
        flags.insert("--src-dir".to_owned(), Some(src_path.clone()));
        let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

        let thread_pool = ThreadPool::new(4);

        let ts = timestamp();
        let mut cache_state = cache::load(cfg.cache_path, &thread_pool, ts);

        let mut interner = Interner::new();

        let info = scan(cfg, &mut interner, &thread_pool, 0, &mut cache_state, ts);

        assert_eq!(info.map.len(), 3);

        let core = entry_index(&info, &mut interner, "core");
        check_modules(&info, &mut interner, core, &["core"], &[]);
        check_part_list(&info.lib[core], &mut interner, &["base_utils"]);
        check_part_list(&info.bin[core], &mut interner, &[]);
        check_part_list(&info.test[core], &mut interner, &["core"]);
        check_deps(&info, &mut interner, core, &[]);

        let mid = entry_index(&info, &mut interner, "mid");
        check_modules(&info, &mut interner, mid, &["mid"], &[]);
        check_part_list(&info.lib[mid], &mut interner, &[]);
        check_part_list(&info.bin[mid], &mut interner, &["run_42"]);
        check_part_list(&info.test[mid], &mut interner, &["mid"]);
        check_deps(&info, &mut interner, mid, &["core"]);

        let top = entry_index(&info, &mut interner, "top");
        check_modules(
            &info,
            &mut interner,
            top,
            &["top", "extra"],
            &[("top", "extra")],
        );
        check_part_list(&info.lib[top], &mut interner, &[]);
        check_part_list(&info.bin[top], &mut interner, &["top_main"]);
        check_part_list(&info.test[top], &mut interner, &["top"]);
        check_deps(&info, &mut interner, top, &["mid"]);

        check_all_basic_hashes(&info, &interner, cfg.src_path.to_str().unwrap(), 10);
    }
}

fn make_entry_graph(interner: &mut Interner, names: &[&str]) -> (Vec<Str>, HashMap<Str, u32>) {
    let strs: Vec<Str> = names.iter().map(|name| interner.insert(name)).collect();
    let map: HashMap<Str, u32> = strs
        .iter()
        .enumerate()
        .map(|(idx, &s)| (s, u32::try_from(idx).unwrap()))
        .collect();
    (strs, map)
}

fn make_dep_matrix(num_parts: usize, deps: &[(usize, usize)]) -> Vec<bool> {
    let mut matrix = vec![false; num_parts * num_parts];
    for &(part, dep) in deps.iter() {
        matrix[part * num_parts + dep] = true;
    }
    matrix
}

#[test]
fn test_entry_cycle_check_accepts_dag() {
    let mut interner = Interner::new();
    // a -> b, c; b -> d; c -> d; e is disconnected.
    let (names, map) = make_entry_graph(&mut interner, &["a", "b", "c", "d", "e"]);
    let deps = vec![
        vec![names[1], names[2]],
        vec![names[3]],
        vec![names[3]],
        Vec::new(),
        Vec::new(),
    ];

    check_cyclic_dependencies_between_entries(&interner, &names, &map, &deps);
}

#[test]
#[should_panic(expected = "dependency cycle between entries")]
fn test_entry_cycle_check_rejects_cycle() {
    let mut interner = Interner::new();
    // d is independent and visited first. a -> b -> c -> a is a cycle.
    let (names, map) = make_entry_graph(&mut interner, &["d", "a", "b", "c"]);
    let deps = vec![Vec::new(), vec![names[2]], vec![names[3]], vec![names[1]]];

    check_cyclic_dependencies_between_entries(&interner, &names, &map, &deps);
}

#[test]
fn test_module_cycle_check_accepts_dag() {
    let mut interner = Interner::new();
    let names = ["root", "left", "right", "common"];
    let parts: Vec<Str> = names.iter().map(|name| interner.insert(name)).collect();
    // root -> left, right, common; left/right -> common.
    let matrix = make_dep_matrix(4, &[(0, 1), (0, 2), (0, 3), (1, 3), (2, 3)]);
    let entry_name = interner.insert("entry");

    check_cyclic_dependencies_between_module_parts(&interner, entry_name, &parts, &matrix);
}

#[test]
#[should_panic(expected = "module dependency cycle")]
fn test_module_cycle_check_rejects_cycle() {
    let mut interner = Interner::new();
    let names = ["a", "b", "c"];
    let parts: Vec<Str> = names.iter().map(|name| interner.insert(name)).collect();
    // a -> b -> c -> a.
    let matrix = make_dep_matrix(3, &[(0, 1), (1, 2), (2, 0)]);
    let entry_name = interner.insert("entry");

    check_cyclic_dependencies_between_module_parts(&interner, entry_name, &parts, &matrix);
}
