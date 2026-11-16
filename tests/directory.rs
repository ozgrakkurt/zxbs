use std::path::{Path, PathBuf};

use zxbs::directory::{list_dir, remove_dir_all};
use zxbs::thread_pool::ThreadPool;

fn sorted(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort_unstable();
    paths
}

#[test]
fn test_list_and_remove_directory_tree() {
    let folder = PathBuf::from(format!("/tmp/zxbs-tests/{}", rand::random::<u128>()));
    let root = folder.join("root");

    std::fs::create_dir_all(root.join("a").join("b")).unwrap();
    std::fs::create_dir(root.join("empty")).unwrap();
    std::fs::write(root.join("file_at_root"), b"x").unwrap();
    std::fs::write(root.join("a").join("file_in_a"), b"y").unwrap();

    let (dirs, files) = list_dir(&root);
    assert_eq!(sorted(dirs), vec![root.join("a"), root.join("empty")],);
    assert_eq!(sorted(files), vec![root.join("file_at_root")]);

    let (dirs, files) = list_dir(&root.join("a"));
    assert_eq!(sorted(dirs), vec![root.join("a").join("b")]);
    assert_eq!(sorted(files), vec![root.join("a").join("file_in_a")]);

    // An empty directory has no entries.
    let (dirs, files) = list_dir(&root.join("empty"));
    assert!(dirs.is_empty());
    assert!(files.is_empty());

    let thread_pool = ThreadPool::new(4);
    remove_dir_all(&root, &thread_pool);

    assert!(!root.exists());

    // Removing a directory that doesn't exist anymore is a no-op.
    remove_dir_all(&root, &thread_pool);
    assert!(!root.exists());

    std::fs::remove_dir_all(Path::new(&folder)).unwrap();
}
