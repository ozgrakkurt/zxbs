#![allow(clippy::single_element_loop)]

use std::collections::HashMap;
use std::path::Path;

use zxbs::config::Config;
use zxbs::interner::Interner;
use zxbs::scan::{BuildInfo, scan};

mod make_folders;
use make_folders::{make_baselib, make_iolib, make_myapp};

#[tokio::test]
async fn test_empty() {
    let folder_name: u128 = rand::random();
    let build_path = format!("/tmp/{}/build", folder_name);
    let src_path = format!("/tmp/{}/src", folder_name);
    std::fs::create_dir_all(&build_path).unwrap();
    std::fs::create_dir_all(&src_path).unwrap();

    let mut flags = HashMap::new();
    flags.insert("--build-dir".to_owned(), build_path.clone());
    flags.insert("--src-dir".to_owned(), src_path.clone());
    let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

    let mut interner = Interner::new();

    let info = scan(cfg, &mut interner).await;

    assert!(info.map.is_empty());
    assert!(info.name.is_empty());
    assert!(info.module.is_empty());
    assert!(info.mod_dep_matrix.is_empty());
    assert!(info.lib.is_empty());
    assert!(info.bin.is_empty());
    assert!(info.test.is_empty());
    assert!(info.deps.is_empty());
}

fn check_baselib_info(info: &BuildInfo, interner: &mut Interner) {
    let name_idx = interner.insert("base");
    let i = info.name.iter().position(|&n| n == name_idx).unwrap();

    assert_eq!(
        usize::try_from(*info.map.get(&name_idx).unwrap()).unwrap(),
        i,
    );

    let mut part_idx_map: HashMap<&'static str, usize> = HashMap::new();

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

    let mut part_idx_map: HashMap<&'static str, usize> = HashMap::new();

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

    let mut part_idx_map: HashMap<&'static str, usize> = HashMap::new();

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

#[tokio::test]
async fn test_simple() {
    let folder_name: u128 = rand::random();
    let build_path = format!("/tmp/{}/build", folder_name);
    let src_path = format!("/tmp/{}/src", folder_name);
    tokio::fs::create_dir_all(&build_path).await.unwrap();
    tokio::fs::create_dir_all(&src_path).await.unwrap();

    // create folders/files
    make_baselib(Path::new(&src_path)).await;
    make_iolib(Path::new(&src_path)).await;
    make_myapp(Path::new(&src_path)).await;

    let mut flags = HashMap::new();
    flags.insert("--build-dir".to_owned(), build_path.clone());
    flags.insert("--src-dir".to_owned(), src_path.clone());
    let cfg: &'static Config = Box::leak(Box::new(Config::parse_config(flags)));

    let mut interner = Interner::new();

    let info = scan(cfg, &mut interner).await;

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

    tokio::fs::remove_dir_all(format!("/tmp/{}", folder_name))
        .await
        .unwrap();
}
