use crate::config::Config;
use crate::interner::Interner;
use crate::scan::BuildInfo;

pub fn setup_build_dir(config: &Config, info: &BuildInfo, interner: &mut Interner) {
    // ignore error since it might not exist
    let _ = std::fs::remove_dir_all(config.build_path);
    // recreate
    std::fs::create_dir_all(config.build_path).unwrap();

    let modules_build_path = config.build_path.join("mod");
    std::fs::create_dir(modules_build_path).unwrap();

    std::fs::create_dir_all(config.cache_path).unwrap();

    for &entry_name_idx in info.name.iter() {
        let entry_name = interner.get(entry_name_idx);

        let entry_build_path = config.build_path.join(entry_name);
        std::fs::create_dir(&entry_build_path).unwrap();

        let lib_build_path = entry_build_path.join("lib");
        std::fs::create_dir(lib_build_path).unwrap();

        let bin_build_path = entry_build_path.join("bin");
        std::fs::create_dir(bin_build_path).unwrap();

        let test_build_path = entry_build_path.join("test");
        std::fs::create_dir(&test_build_path).unwrap();

        let test_stamp_path = test_build_path.join("stamp");
        std::fs::create_dir(test_stamp_path).unwrap();
    }
}
