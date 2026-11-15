#![allow(clippy::single_element_loop)]

use zxbs::cache;
use zxbs::config::{Config, Subcommand};
use zxbs::hash::{HashMap, hash_ordered};
use zxbs::interner::Interner;
use zxbs::scan::{BuildInfo, scan};
use zxbs::thread_pool::ThreadPool;
use zxbs::timestamp::timestamp;

mod make_folders;
use make_folders::{make_baselib, make_iolib, make_myapp};

#[test]
fn test_empty() {
    let folder = format!("/tmp/opencode/zxbs-tests/{}", rand::random::<u128>());
    let build_path = format!("{}/build", folder);
    let src_path = format!("{}/src", folder);
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

    std::fs::remove_dir_all(&folder).unwrap();
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

fn check_all_basic_hashes(info: &BuildInfo, interner: &Interner, src_path: &str) {
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

    assert_eq!(num_checked, 22);
    assert_eq!(info.basic_hash.len(), 22);
}

#[test]
fn test_simple() {
    let folder = format!("/tmp/opencode/zxbs-tests/{}", rand::random::<u128>());
    let build_path = format!("{}/build", folder);
    let src_path = format!("{}/src", folder);
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

    check_all_basic_hashes(&info, &interner, cfg.src_path.to_str().unwrap());

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

    std::fs::remove_dir_all(&folder).unwrap();
}
