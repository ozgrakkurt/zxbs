use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::interner::Interner;
use crate::scan::BuildInfo;
use crate::thread_pool::{JoinSet, ThreadPool};

// Lists a directory, splitting its entries into files and directories.
//
// Returns (paths of directories, paths of files)
pub fn list_dir(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();

    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let file_type = entry.file_type().unwrap();

        if file_type.is_dir() {
            dirs.push(path);
        } else if file_type.is_file() {
            files.push(path);
        } else {
            panic!(
                "Unexpected file kind at `{}` while listing directories.",
                path.display()
            );
        }
    }

    (dirs, files)
}

// Removes `root` and everything inside it. This is similar to
// `std::fs::remove_dir_all`, except that the files are unlinked in parallel.
//
// The tree is walked level by level. For each level, one task per directory
// lists its contents and one task per file unlinks it. Both kinds of tasks run
// in the same JoinSet, so files start getting deleted while other directories
// at the same level are still being listed. Once every file is gone, the
// directories are removed level by level from the bottom up.
pub fn remove_dir_all(root: &Path, thread_pool: &ThreadPool) {
    if !std::fs::exists(root).unwrap() {
        return;
    }

    let mut ls_join_set: JoinSet<(Vec<PathBuf>, Vec<PathBuf>)> = JoinSet::new();
    let mut delete_join_set: JoinSet<()> = JoinSet::new();

    // Every level of the tree, starting with `root` itself. The levels are
    // removed in reverse order once all files have been deleted.
    let mut levels: Vec<Vec<PathBuf>> = Vec::new();
    let mut current_level = vec![root.to_path_buf()];

    // Delete every file, one level of directories at a time. The loop stops
    // once there are no more subdirectories to list, at which point every file
    // in the tree has been scheduled for deletion.
    while !current_level.is_empty() {
        levels.push(current_level.clone());

        for dir in current_level.drain(..) {
            ls_join_set.spawn(thread_pool, move || list_dir(&dir));
        }

        let mut next_level = Vec::new();
        while let Some((dirs, files)) = ls_join_set.join_next() {
            next_level.extend_from_slice(&dirs);

            for file in files {
                delete_join_set.spawn(thread_pool, move || {
                    std::fs::remove_file(file).unwrap();
                });
            }
        }

        current_level = next_level;
    }

    // This is just a mental model check. Not a problem if it isn't empty at this point but
    // check anyway.
    assert!(ls_join_set.is_empty());

    delete_join_set.join_all();

    // Every file is gone, so the directories can be removed. Going from the
    // deepest level to `root` guarantees that each directory is empty by the
    // time it is removed.
    for level in levels.into_iter().rev() {
        for dir in level {
            delete_join_set.spawn(thread_pool, move || {
                std::fs::remove_dir(dir).unwrap();
            });
        }

        delete_join_set.join_all();
    }
}

pub fn create_build_directories(
    config: &Config,
    info: &BuildInfo,
    interner: &Interner,
    thread_pool: &ThreadPool,
) {
    std::fs::create_dir_all(config.build_path).unwrap();
    std::fs::create_dir_all(config.cache_path).unwrap();

    let mut join_set: JoinSet<()> = JoinSet::new();

    // Directories are created level by level with `create_dir`, since each
    // level depends on the previous one existing. A single JoinSet is reused
    // and joined between levels so every directory within a level is created
    // in parallel.

    // Level 1: children of the build directory.
    let modules_build_path = config.build_path.join("mod");
    join_set.spawn(thread_pool, move || {
        std::fs::create_dir(modules_build_path).unwrap();
    });

    for &entry_name_idx in info.name.iter() {
        let entry_name = interner.get(entry_name_idx);
        let entry_build_path = config.build_path.join(entry_name);
        join_set.spawn(thread_pool, move || {
            std::fs::create_dir(entry_build_path).unwrap();
        });
    }

    join_set.join_all();

    // Level 2: children of each entry directory.
    for &entry_name_idx in info.name.iter() {
        let entry_name = interner.get(entry_name_idx);

        let lib_build_path = config.build_path.join(entry_name).join("lib");
        join_set.spawn(thread_pool, move || {
            std::fs::create_dir(lib_build_path).unwrap();
        });

        let bin_build_path = config.build_path.join(entry_name).join("bin");
        join_set.spawn(thread_pool, move || {
            std::fs::create_dir(bin_build_path).unwrap();
        });

        let test_build_path = config.build_path.join(entry_name).join("test");
        join_set.spawn(thread_pool, move || {
            std::fs::create_dir(test_build_path).unwrap();
        });
    }

    join_set.join_all();

    // Level 3: children of each entry's test directory.
    for &entry_name_idx in info.name.iter() {
        let entry_name = interner.get(entry_name_idx);
        let test_stamp_path = config
            .build_path
            .join(entry_name)
            .join("test")
            .join("stamp");
        join_set.spawn(thread_pool, move || {
            std::fs::create_dir(test_stamp_path).unwrap();
        });
    }

    join_set.join_all();
}

pub fn setup_build_directories(
    config: &Config,
    info: &BuildInfo,
    interner: &Interner,
    thread_pool: &ThreadPool,
) {
    // Remove the previous build directory. We parallelize this instead of
    // just doing a single remove_dir_all because unlinking all those build
    // files is actually a bottleneck here.
    remove_dir_all(config.build_path, thread_pool);

    create_build_directories(config, info, interner, thread_pool);
}
