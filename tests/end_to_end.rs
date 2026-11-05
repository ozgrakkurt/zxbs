use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use config::{Config, Subcommand};
use interner::Interner;
use zxbs::{build, config, interner, plan, scan};

mod make_folders;
use make_folders::{make_baselib, make_iolib, make_myapp};

#[tokio::test]
async fn test_smoke() {
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

    // run entire build and then check that the files actually exist

    let info = scan::scan(cfg, &mut interner).await;
    let tasks = plan::plan(cfg, &Subcommand::TestAll, &mut interner, &info);
    {
        let build = build::Build::new(None, &mut interner, tasks, Instant::now());
        build.await;
    }

    tokio::fs::remove_dir_all(format!("/tmp/{}", folder_name))
        .await
        .unwrap();
}
