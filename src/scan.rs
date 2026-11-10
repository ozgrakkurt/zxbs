use std::path::{Path, PathBuf};

use crate::check::check_name;
use crate::config::{Config, Subcommand};
use crate::hash::{Hash, HashMap, hash_ordered};
use crate::interner::{Interner, Str};
use crate::thread_pool::{JoinSet, ThreadPoolHandle};

pub fn make_module_name(root_name: &str, part_name: &str) -> String {
    if root_name == part_name {
        root_name.to_owned()
    } else {
        format!("{}:{}", root_name, part_name)
    }
}

struct EntryInfo {
    name: String,
    module: Vec<String>,
    mod_dep_matrix: Vec<bool>,
    lib: Vec<String>,
    bin: Vec<String>,
    test: Vec<String>,
    deps: Vec<String>,
    // Maps source file path -> basic hash of the file at that path.
    // Basic hash as opposed to the "deep" hash.
    //
    // Deep hash means the hash is an
    // aggregate of everything that the file depends on including build flags and
    // build system executable.
    //
    // Basic hash means the hash is just the hash of the source file contents combined with the hash of the source file path.
    //
    // The more complicated "deep" hash will be calculated based on these basic hashes
    // at the "plan" stage after the scanning is finished.
    //
    // Hashes are used for implementing caching.
    basic_hash: HashMap<String, Hash>,
}

#[derive(Default)]
pub struct BuildInfo {
    // maps entry_name => entry_index
    pub map: HashMap<Str, u32>,
    pub name: Vec<Str>,
    pub module: Vec<Vec<Str>>,
    // matrix of mod_count * mod_count.
    // matrix[i * mod_count + j] is true if mod no i depends on mod no j.
    pub mod_dep_matrix: Vec<Vec<bool>>,
    pub lib: Vec<Vec<Str>>,
    pub bin: Vec<Vec<Str>>,
    pub test: Vec<Vec<Str>>,
    pub deps: Vec<Vec<Str>>,
    // As explained in EntryInfo. This field is just an aggregate of all basic hashes in individual
    // EntryInfos.
    pub basic_hash: HashMap<Str, Hash>,
    pub self_exe_hash: Hash,
}

impl BuildInfo {
    pub fn check_subcommand(&self, interner: &mut Interner, subcommand: &Subcommand) {
        let (entry_name, part_name, is_test) = match subcommand {
            Subcommand::RunBinary {
                entry_name,
                bin_name,
                ..
            } => (entry_name, bin_name, false),
            Subcommand::TestAll => return,
            Subcommand::Test {
                entry_name,
                test_name,
            } => (entry_name, test_name, true),
            Subcommand::Build => return,
            Subcommand::Fmt => return,
        };

        let entry = interner.insert(entry_name);
        let part = interner.insert(part_name);

        let eidx = match self.map.get(&entry) {
            Some(eidx) => usize::try_from(*eidx).unwrap(),
            None => panic!(
                "Subcommand argument `{}:{}` has entry as `{}`. But an entry with this name wasn't found after scanning.",
                entry_name, part_name, entry_name
            ),
        };

        let kind_name = if is_test { "test" } else { "binary" };

        let list = if is_test {
            &self.test[eidx]
        } else {
            &self.bin[eidx]
        };

        for p in list.iter() {
            if *p == part {
                return;
            }
        }

        panic!(
            "Subcommand argument has entry `{}:{}. But the entry `{}` doesn't have a {} named `{}`",
            entry_name, part_name, entry_name, kind_name, part_name
        );
    }

    fn push_entry_info(&mut self, interner: &mut Interner, entry_info: EntryInfo) {
        let index = u32::try_from(self.name.len()).unwrap();
        let name = interner.insert(&entry_info.name);
        assert!(self.map.insert(name, index).is_none());
        self.name.push(name);
        self.module.push(
            entry_info
                .module
                .iter()
                .map(|s| interner.insert(s))
                .collect(),
        );
        self.mod_dep_matrix.push(entry_info.mod_dep_matrix);
        self.lib
            .push(entry_info.lib.iter().map(|s| interner.insert(s)).collect());
        self.bin
            .push(entry_info.bin.iter().map(|s| interner.insert(s)).collect());
        self.test
            .push(entry_info.test.iter().map(|s| interner.insert(s)).collect());
        self.deps
            .push(entry_info.deps.iter().map(|s| interner.insert(s)).collect());

        for (path, hash) in entry_info.basic_hash.iter() {
            assert!(
                self.basic_hash
                    .insert(interner.insert(path), *hash)
                    .is_none()
            );
        }
    }
}

pub struct CppmDeps {
    pub deps: Vec<String>,
    pub provides: Vec<String>,
}

pub fn parse_p1689(contents: &[u8]) -> CppmDeps {
    let root = serde_json::from_slice::<serde_json::Value>(contents).unwrap();
    let root = root.as_object().unwrap();

    let mut deps = Vec::new();
    let mut provides = Vec::new();

    if let Some(rules) = root.get("rules") {
        for rule in rules.as_array().unwrap().iter() {
            let rule = rule.as_object().unwrap();

            if let Some(prov) = rule.get("provides") {
                for p in prov.as_array().unwrap().iter() {
                    let name = p
                        .as_object()
                        .unwrap()
                        .get("logical-name")
                        .unwrap()
                        .as_str()
                        .unwrap();
                    provides.push(name.to_owned());
                }
            }

            if let Some(reqs) = rule.get("requires") {
                for p in reqs.as_array().unwrap().iter() {
                    let name = p
                        .as_object()
                        .unwrap()
                        .get("logical-name")
                        .unwrap()
                        .as_str()
                        .unwrap();
                    deps.push(name.to_owned());
                }
            }
        }
    }

    CppmDeps { deps, provides }
}

fn scan_deps(config: &Config, path: &Path) -> CppmDeps {
    let mut cmd = std::process::Command::new(config.clang_scan_deps);
    cmd.arg("-format=p1689")
        .arg("--")
        .arg(config.cxx)
        .args(config.cxxflags)
        .arg("-c")
        .arg(path);
    let output = match cmd.output() {
        Ok(out) => out,
        Err(e) => {
            panic!(
                "Failed to run scan deps command for {}: {}",
                path.display(),
                e
            );
        }
    };
    if !output.status.success() {
        panic!(
            "Failed clang_scan_deps for {}:\n{}",
            path.display(),
            str::from_utf8(&output.stderr).unwrap()
        );
    }

    parse_p1689(output.stdout.as_slice())
}

pub fn parse_deps_file(contents: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in contents.trim().split('\n') {
        let line = line.trim();
        if !line.is_empty() {
            check_name(line);
            deps.push(line.to_owned());
        }
    }
    deps
}

fn read_deps_file(path: PathBuf) -> Vec<String> {
    let contents = String::from_utf8(std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read deps file at: {}. Error={}",
            path.display(),
            e
        )
    }))
    .unwrap();
    parse_deps_file(contents.as_str())
}

pub fn parse_file_name<'name>(
    path: &Path,
    expected_extension: &str,
    file_name: &'name str,
) -> &'name str {
    let parts: Vec<_> = file_name.split(".").collect();
    assert_eq!(
        parts.len(),
        2,
        "invalid file name: {}, expected extension: {}. Path={}",
        file_name,
        expected_extension,
        path.display(),
    );
    let name = parts[0];
    let extension = parts[1];

    check_name(name);

    assert_eq!(
        expected_extension,
        extension,
        "unexpected extension on filename: {}, expected extension: {}. Path={}",
        file_name,
        expected_extension,
        path.display(),
    );

    name
}

fn scan_subfolder(expected_extension: &'static str, path: &Path) -> Vec<String> {
    let dir = match std::fs::read_dir(path) {
        Ok(v) => v,
        Err(e) => {
            panic!("Failed to read_dir on '{}'. Error: {}", path.display(), e);
        }
    };
    let mut names = Vec::new();
    for file in dir {
        let file = file.unwrap();
        let name = parse_file_name(
            &file.path(),
            expected_extension,
            file.file_name().to_str().unwrap(),
        )
        .to_owned();
        check_name(&name);
        names.push(name);
    }
    names
}

struct ObjectFolderInfo {
    parts: Vec<String>,
    deps: Vec<Vec<String>>,
}

// For scanning bin/test/lib folders for cc files and scanning their module dependencies
fn scan_object_folder(
    config: &'static Config,
    path: &Path,
    thread_pool: ThreadPoolHandle<'static>,
) -> ObjectFolderInfo {
    let parts = scan_subfolder("cc", path);

    let mut futs = JoinSet::new();

    for part in parts {
        let cppm_path = path.join(format!("{}.cc", part));
        futs.spawn(thread_pool, move || {
            let cppm_deps = scan_deps(config, &cppm_path);

            if !cppm_deps.provides.is_empty() {
                panic!(
                    "Didn't expect {} to provide any module interface. But it provides: {:?}",
                    cppm_path.display(),
                    cppm_deps.provides
                );
            }

            (part, cppm_deps.deps)
        });
    }

    let dep_infos = futs.join_all();

    let mut parts = Vec::new();
    let mut deps = Vec::new();

    for info in dep_infos {
        parts.push(info.0);
        deps.push(info.1);
    }

    ObjectFolderInfo { parts, deps }
}

fn check_object_folder_info(
    entry_name: &str,
    self_module_parts: &[String],
    ext_deps: &[String],
    info: &ObjectFolderInfo,
) {
    assert_eq!(info.parts.len(), info.deps.len());
    for i in 0..info.parts.len() {
        for dep in info.deps[i].iter() {
            if dep.contains(':') {
                let parts: Vec<&str> = dep.split(':').collect();
                if parts.len() != 2 {
                    panic!(
                        "invalid dependency name '{}' in part '{}' of entry '{}'. Failed to parse.",
                        dep, info.parts[i], entry_name
                    );
                }
                if parts[0] != entry_name {
                    panic!(
                        "invalid dependency name '{}' in part '{}' of entry '{}'. Expected root part to be self entry name since depending directly on partitions of external modules isn't supported.",
                        dep, info.parts[i], entry_name
                    );
                }
                if !self_module_parts.contains(&parts[1].to_owned()) {
                    panic!(
                        "invalid dependency name '{}' in part '{}' of entry '{}'. Parition isn't a member of the partitions of current entry. Available partitions are: {:?}",
                        dep, info.parts[i], entry_name, self_module_parts,
                    );
                }
            } else if dep != entry_name && !ext_deps.contains(dep) {
                panic!(
                    "Dependency '{}' of part '{}' of entry '{}' is invalid. External dependencies are: {:?}",
                    dep, info.parts[i], entry_name, ext_deps
                );
            }
        }
    }
}

fn spawn_collect_basic_hashes(
    config: &'static Config,
    kind_name: &str,
    entry_name: &str,
    names: &[String],
    extension: &str,
    concurrency: usize,
    join_set: &mut JoinSet<(String, Hash)>,
    basic_hash: &mut HashMap<String, Hash>,
    thread_pool: ThreadPoolHandle<'static>,
) {
    for part_name in names.iter() {
        // don't go over the concurrency limit
        while join_set.len() >= concurrency {
            let (p, hash) = join_set.join_next().unwrap();

            assert!(basic_hash.insert(p, hash).is_none());
        }

        let path = config
            .src_path
            .join(entry_name)
            .join(kind_name)
            .join(format!("{}.{}", part_name, extension));
        let path = path.to_str().unwrap().to_owned();

        join_set.spawn(thread_pool, move || {
            let data = std::fs::read(&path).unwrap();
            let hash = hash_ordered(&[path.as_bytes(), data.as_slice()]);
            (path, hash)
        });
    }
}

fn scan_entry(
    config: &'static Config,
    path: &Path,
    thread_pool: ThreadPoolHandle<'static>,
) -> EntryInfo {
    let entry_name = path.file_name().unwrap();
    let entry_name = entry_name.to_str().unwrap().to_owned();
    check_name(&entry_name);

    let deps = thread_pool.spawn({
        let p = path.join("deps");
        move || read_deps_file(p)
    });

    let mod_parts = thread_pool.spawn({
        let path = path.join("mod");
        move || scan_subfolder("cppm", &path)
    });
    let lib = thread_pool.spawn({
        let path = path.join("lib");
        move || scan_object_folder(config, &path, thread_pool.next_layer())
    });
    let test = thread_pool.spawn({
        let path = path.join("test");
        move || scan_object_folder(config, &path, thread_pool.next_layer())
    });
    let bin = thread_pool.spawn({
        let path = path.join("bin");
        move || scan_object_folder(config, &path, thread_pool.next_layer())
    });

    let deps = deps.join();
    let mod_parts = mod_parts.join();
    let n_mod_parts = mod_parts.len();

    // has to have root module
    assert!(
        mod_parts.contains(&entry_name),
        "root module of entry {} not found",
        entry_name,
    );

    let mod_path = path.join("mod");

    let concurrency = num_cpus::get();
    let mut join_set = JoinSet::new();
    let mut mod_dep_names = vec![Vec::new(); mod_parts.len()];
    for (pos, part_name) in mod_parts.iter().enumerate() {
        while join_set.len() >= concurrency {
            let (idx, deps) = join_set.join_next().unwrap();
            mod_dep_names[idx] = deps;
        }
        let part_name = part_name.to_owned();
        let cppm_path = mod_path.join(format!("{}.cppm", part_name));
        let expected_provides = make_module_name(&entry_name, &part_name);
        join_set.spawn(thread_pool, move || {
            let cppm_deps = scan_deps(config, &cppm_path);

            if cppm_deps.provides.len() != 1
                || cppm_deps.provides[0].as_str() != expected_provides.as_str()
            {
                panic!(
                    "expected `{}` to only provide `{}`, but it provides: `{:?}``",
                    cppm_path.display(),
                    expected_provides,
                    cppm_deps.provides
                );
            }

            (pos, cppm_deps.deps)
        });
    }
    while let Some((idx, deps)) = join_set.join_next() {
        mod_dep_names[idx] = deps;
    }

    let mut mod_dep_matrix = vec![false; n_mod_parts * n_mod_parts];
    for (part_pos, part_name) in mod_parts.iter().enumerate() {
        let mod_dep_names = &mod_dep_names[part_pos];
        for mod_dep_name in mod_dep_names.iter() {
            if mod_dep_name.contains(':') {
                let parts: Vec<_> = mod_dep_name.split(':').collect();
                assert_eq!(parts.len(), 2);
                check_name(parts[0]);
                check_name(parts[1]);
                let part0 = parts[0];
                let part1 = parts[1];

                assert_eq!(
                    part0, entry_name,
                    "part of entry {} doesn't define a module under it. It instead defines under {}",
                    entry_name, part0,
                );
                assert_ne!(
                    part1, part_name,
                    "part {} of entry {} doesn't define expected module {}:{}",
                    part_name, entry_name, entry_name, part_name,
                );

                let dep_pos = mod_parts
                    .iter()
                    .position(|nidx| *nidx == part1)
                    .unwrap_or_else(|| {
                        panic!(
                            "dependency with name `{}` of `{}:{}` can't be found",
                            mod_dep_name, entry_name, part_name,
                        )
                    });

                mod_dep_matrix[part_pos * n_mod_parts + dep_pos] = true;
                assert!(
                    !mod_dep_matrix[dep_pos * n_mod_parts + part_pos],
                    "cyclic dependency between {} and {} in entry {}",
                    part_name,
                    mod_dep_name,
                    entry_name,
                );
            } else {
                let is_direct_dep = deps.contains(mod_dep_name);
                assert!(
                    is_direct_dep,
                    "Module file {}:{} depends on {} but it isn't listed in the deps file of {}",
                    entry_name, part_name, mod_dep_name, entry_name,
                );
            }
        }
    }

    let lib = lib.join();
    check_object_folder_info(&entry_name, &mod_parts, &deps, &lib);

    let test = test.join();

    let bin = bin.join();

    // Collect the basic hashes of all source files in parallel.
    let mut basic_hash = HashMap::<String, Hash>::default();
    let mut join_set = JoinSet::new();
    spawn_collect_basic_hashes(
        config,
        "lib",
        &entry_name,
        &lib.parts,
        "cc",
        concurrency,
        &mut join_set,
        &mut basic_hash,
        thread_pool,
    );
    spawn_collect_basic_hashes(
        config,
        "test",
        &entry_name,
        &test.parts,
        "cc",
        concurrency,
        &mut join_set,
        &mut basic_hash,
        thread_pool,
    );
    spawn_collect_basic_hashes(
        config,
        "bin",
        &entry_name,
        &bin.parts,
        "cc",
        concurrency,
        &mut join_set,
        &mut basic_hash,
        thread_pool,
    );
    spawn_collect_basic_hashes(
        config,
        "mod",
        &entry_name,
        &mod_parts,
        "cppm",
        concurrency,
        &mut join_set,
        &mut basic_hash,
        thread_pool,
    );

    while let Some((path, hash)) = join_set.join_next() {
        assert!(basic_hash.insert(path, hash).is_none());
    }

    EntryInfo {
        name: entry_name,
        module: mod_parts,
        mod_dep_matrix,
        lib: lib.parts,
        bin: bin.parts,
        test: test.parts,
        deps,
        basic_hash,
    }
}

pub fn scan(
    config: &'static Config,
    interner: &mut Interner,
    thread_pool: ThreadPoolHandle<'static>,
) -> BuildInfo {
    let dir = std::fs::read_dir(config.src_path).unwrap();

    let mut info = BuildInfo::default();

    let mut join_set = JoinSet::new();
    let concurrency = num_cpus::get();

    for entry in dir {
        let entry = entry.unwrap();
        while join_set.len() >= concurrency {
            let entry_info = join_set.join_next().unwrap();
            info.push_entry_info(interner, entry_info);
        }
        join_set.spawn(thread_pool, move || {
            scan_entry(config, &entry.path(), thread_pool.next_layer())
        });
    }

    while let Some(entry_info) = join_set.join_next() {
        info.push_entry_info(interner, entry_info);
    }

    {
        const SELF_EXE_PATH: &str = "/proc/self/exe";
        let self_exe_bytes = std::fs::read(SELF_EXE_PATH).unwrap();
        info.self_exe_hash = hash_ordered(&[SELF_EXE_PATH.as_bytes(), self_exe_bytes.as_slice()]);
    }

    info
}
