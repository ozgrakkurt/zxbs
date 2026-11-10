use crate::config::Config;
use crate::interner::Interner;
use crate::scan::BuildInfo;

pub async fn setup_build_dir(config: &Config, info: &BuildInfo, interner: &mut Interner) {
    // ignore error since it might not exist
    let _ = tokio::fs::remove_dir_all(config.build_path).await;
    // recreate
    tokio::fs::create_dir_all(config.build_path).await.unwrap();

    let modules_build_path = config.build_path.join("mod");
    tokio::fs::create_dir(modules_build_path).await.unwrap();

    tokio::fs::create_dir_all(config.cache_path).await.unwrap();

    for &entry_name_idx in info.name.iter() {
        let entry_name = interner.get(entry_name_idx);

        let entry_build_path = config.build_path.join(entry_name);
        tokio::fs::create_dir(&entry_build_path).await.unwrap();

        let lib_build_path = entry_build_path.join("lib");
        tokio::fs::create_dir(lib_build_path).await.unwrap();

        let bin_build_path = entry_build_path.join("bin");
        tokio::fs::create_dir(bin_build_path).await.unwrap();

        let test_build_path = entry_build_path.join("test");
        tokio::fs::create_dir(&test_build_path).await.unwrap();

        let test_stamp_path = test_build_path.join("stamp");
        tokio::fs::create_dir(test_stamp_path).await.unwrap();
    }
}
