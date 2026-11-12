use std::path::{Path, PathBuf};

use crate::check::check_name;
use crate::config::{Config, Subcommand};
use crate::hash::{Hash, HashMap, hash_bytes, hash_ordered, mix_hashes};
use crate::interner::{Interner, Str};
use crate::thread_pool::{JoinSet, ThreadPool};

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

fn run_scan_deps(config: &Config, path: &Path) -> Vec<u8> {
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

    output.stdout
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

// The layout of a single entry that can be discovered just by listing its
// directories and reading its deps file. This is the "cheap" part of scanning
// and is done for every entry in parallel in the first phase.
struct EntryListing {
    name: String,
    module: Vec<String>,
    lib: Vec<String>,
    test: Vec<String>,
    bin: Vec<String>,
    deps: Vec<String>,
}

// A single piece of an entry's layout produced during the first phase. Each
// entry has one task per piece (deps + mod/lib/test/bin folders) so that
// listing is fully parallel without any nested concurrency.
enum ListingTask {
    Deps { entry: usize, deps: Vec<String> },
    Mod { entry: usize, parts: Vec<String> },
    Lib { entry: usize, parts: Vec<String> },
    Test { entry: usize, parts: Vec<String> },
    Bin { entry: usize, parts: Vec<String> },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Mod,
    Lib,
    Test,
    Bin,
}

impl FileKind {
    fn folder(self) -> &'static str {
        match self {
            FileKind::Mod => "mod",
            FileKind::Lib => "lib",
            FileKind::Test => "test",
            FileKind::Bin => "bin",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            FileKind::Mod => "cppm",
            _ => "cc",
        }
    }
}

// A single source file that needs to be scanned with clang-scan-deps (or read
// from the cache) and hashed.
struct ScanTarget {
    entry: usize,
    kind: FileKind,
    part: usize,
    path: String,
}

// The hash used to key the cached clang-scan-deps output of a source file. It
// is derived from the file's basic hash (so it changes when the file changes)
// and the build's base hash (so it changes when the build configuration
// changes), mixed with a constant marker so it can't collide with the "deep"
// hashes used for build artifacts in the same cache.
fn clang_scan_deps_cache_hash(base_hash: Hash, basic_hash: Hash) -> Hash {
    mix_hashes(
        mix_hashes(base_hash, basic_hash),
        hash_bytes(b"clang-scan-deps-marker"),
    )
}

fn check_provides(
    entry_name: &str,
    kind: FileKind,
    part_name: &str,
    path: &str,
    cppm_deps: CppmDeps,
) -> Vec<String> {
    match kind {
        FileKind::Mod => {
            let expected_provides = make_module_name(entry_name, part_name);
            if cppm_deps.provides.len() != 1
                || cppm_deps.provides[0].as_str() != expected_provides.as_str()
            {
                panic!(
                    "expected `{}` to only provide `{}`, but it provides: `{:?}``",
                    path, expected_provides, cppm_deps.provides
                );
            }
        }
        _ => {
            if !cppm_deps.provides.is_empty() {
                panic!(
                    "Didn't expect {} to provide any module interface. But it provides: {:?}",
                    path, cppm_deps.provides
                );
            }
        }
    }

    cppm_deps.deps
}

fn listing_part_name(listing: &EntryListing, kind: FileKind, part: usize) -> &str {
    match kind {
        FileKind::Mod => &listing.module[part],
        FileKind::Lib => &listing.lib[part],
        FileKind::Test => &listing.test[part],
        FileKind::Bin => &listing.bin[part],
    }
}

fn push_targets(
    targets: &mut Vec<ScanTarget>,
    config: &Config,
    entry: usize,
    entry_name: &str,
    kind: FileKind,
    parts: &[String],
) {
    for (part, part_name) in parts.iter().enumerate() {
        let path = config
            .src_path
            .join(entry_name)
            .join(kind.folder())
            .join(format!("{}.{}", part_name, kind.extension()));
        targets.push(ScanTarget {
            entry,
            kind,
            part,
            path: path.to_str().unwrap().to_owned(),
        });
    }
}

fn build_mod_dep_matrix(
    entry_name: &str,
    mod_parts: &[String],
    deps: &[String],
    mod_dep_names: Vec<Vec<String>>,
) -> Vec<bool> {
    let n_mod_parts = mod_parts.len();
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

    mod_dep_matrix
}

struct ScanResult {
    entry: usize,
    kind: FileKind,
    part: usize,
    deps: Vec<String>,
    path: String,
    basic_hash: Hash,
}

pub fn scan(
    config: &'static Config,
    interner: &mut Interner,
    thread_pool: &ThreadPool,
    base_hash: Hash,
) -> BuildInfo {
    let dir = std::fs::read_dir(config.src_path).unwrap();

    let mut info = BuildInfo::default();

    // First list the source directory to find every entry.
    let mut entries: Vec<(PathBuf, String)> = Vec::new();
    for entry in dir {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        check_name(&name);
        entries.push((path, name));
    }

    // First phase: list each entry's mod/test/bin/lib folders and read its deps
    // file. Each piece is its own task in a single flat JoinSet so nothing is
    // listed serially and there is no nested concurrency.
    let mut join_set = JoinSet::new();
    for (entry, (path, _)) in entries.iter().enumerate() {
        let p = path.join("deps");
        join_set.spawn(thread_pool, move || ListingTask::Deps {
            entry,
            deps: read_deps_file(p),
        });

        let p = path.join("mod");
        join_set.spawn(thread_pool, move || ListingTask::Mod {
            entry,
            parts: scan_subfolder("cppm", &p),
        });

        let p = path.join("lib");
        join_set.spawn(thread_pool, move || ListingTask::Lib {
            entry,
            parts: scan_subfolder("cc", &p),
        });

        let p = path.join("test");
        join_set.spawn(thread_pool, move || ListingTask::Test {
            entry,
            parts: scan_subfolder("cc", &p),
        });

        let p = path.join("bin");
        join_set.spawn(thread_pool, move || ListingTask::Bin {
            entry,
            parts: scan_subfolder("cc", &p),
        });
    }

    let mut deps_list = vec![Vec::new(); entries.len()];
    let mut mod_list = vec![Vec::new(); entries.len()];
    let mut lib_list = vec![Vec::new(); entries.len()];
    let mut test_list = vec![Vec::new(); entries.len()];
    let mut bin_list = vec![Vec::new(); entries.len()];

    while let Some(task) = join_set.join_next() {
        match task {
            ListingTask::Deps { entry, deps } => deps_list[entry] = deps,
            ListingTask::Mod { entry, parts } => mod_list[entry] = parts,
            ListingTask::Lib { entry, parts } => lib_list[entry] = parts,
            ListingTask::Test { entry, parts } => test_list[entry] = parts,
            ListingTask::Bin { entry, parts } => bin_list[entry] = parts,
        }
    }

    let mut listings = Vec::with_capacity(entries.len());
    for (entry, (_, name)) in entries.into_iter().enumerate() {
        let module = std::mem::take(&mut mod_list[entry]);

        // has to have root module
        assert!(
            module.contains(&name),
            "root module of entry {} not found",
            name,
        );

        listings.push(EntryListing {
            name,
            module,
            lib: std::mem::take(&mut lib_list[entry]),
            test: std::mem::take(&mut test_list[entry]),
            bin: std::mem::take(&mut bin_list[entry]),
            deps: std::mem::take(&mut deps_list[entry]),
        });
    }

    // Build the flat list of every source file that needs scanning.
    let mut targets = Vec::new();
    for (entry, listing) in listings.iter().enumerate() {
        push_targets(
            &mut targets,
            config,
            entry,
            &listing.name,
            FileKind::Mod,
            &listing.module,
        );
        push_targets(
            &mut targets,
            config,
            entry,
            &listing.name,
            FileKind::Lib,
            &listing.lib,
        );
        push_targets(
            &mut targets,
            config,
            entry,
            &listing.name,
            FileKind::Test,
            &listing.test,
        );
        push_targets(
            &mut targets,
            config,
            entry,
            &listing.name,
            FileKind::Bin,
            &listing.bin,
        );
    }

    // Second phase: hash and scan every source file in parallel. Each task
    // reads the file and hashes it, then either loads the clang-scan-deps
    // result from the cache or runs clang-scan-deps and stores the result.
    std::fs::create_dir_all(config.cache_path).unwrap();
    let cache_files_path = config.cache_path.to_str().unwrap();
    let mut join_set = JoinSet::new();
    for target in &targets {
        let entry = target.entry;
        let kind = target.kind;
        let part = target.part;
        let path = target.path.clone();
        let entry_name = listings[entry].name.clone();
        let part_name = listing_part_name(&listings[entry], kind, part).to_owned();
        join_set.spawn(thread_pool, move || {
            let data = std::fs::read(&path).unwrap();
            let basic_hash = hash_ordered(&[path.as_bytes(), data.as_slice()]);

            let cache_key = clang_scan_deps_cache_hash(base_hash, basic_hash);
            let cache_file = format!("{}/{:x}", cache_files_path, cache_key);

            let cppm_deps = match std::fs::read(&cache_file) {
                Ok(bytes) => parse_p1689(&bytes),
                Err(_) => {
                    let raw = run_scan_deps(config, Path::new(&path));
                    std::fs::write(&cache_file, &raw).unwrap();
                    parse_p1689(&raw)
                }
            };

            let deps = check_provides(&entry_name, kind, &part_name, &path, cppm_deps);

            ScanResult {
                entry,
                kind,
                part,
                deps,
                path,
                basic_hash,
            }
        });
    }

    // Collect the results of the second phase.
    let mut mod_deps: Vec<Vec<Vec<String>>> = listings
        .iter()
        .map(|listing| vec![Vec::new(); listing.module.len()])
        .collect();
    let mut lib_deps: Vec<Vec<Vec<String>>> = listings
        .iter()
        .map(|listing| vec![Vec::new(); listing.lib.len()])
        .collect();
    let mut basic_hashes: Vec<HashMap<String, Hash>> =
        listings.iter().map(|_| HashMap::default()).collect();

    while let Some(ScanResult {
        entry,
        kind,
        part,
        deps,
        path,
        basic_hash,
    }) = join_set.join_next()
    {
        match kind {
            FileKind::Mod => mod_deps[entry][part] = deps,
            FileKind::Lib => lib_deps[entry][part] = deps,
            FileKind::Test | FileKind::Bin => {}
        }
        assert!(basic_hashes[entry].insert(path, basic_hash).is_none());
    }

    for (entry, listing) in listings.into_iter().enumerate() {
        let mod_dep_matrix = build_mod_dep_matrix(
            &listing.name,
            &listing.module,
            &listing.deps,
            std::mem::take(&mut mod_deps[entry]),
        );

        let lib_info = ObjectFolderInfo {
            parts: listing.lib.clone(),
            deps: std::mem::take(&mut lib_deps[entry]),
        };
        check_object_folder_info(&listing.name, &listing.module, &listing.deps, &lib_info);

        info.push_entry_info(
            interner,
            EntryInfo {
                name: listing.name,
                module: listing.module,
                mod_dep_matrix,
                lib: listing.lib,
                bin: listing.bin,
                test: listing.test,
                deps: listing.deps,
                basic_hash: std::mem::take(&mut basic_hashes[entry]),
            },
        );
    }

    info
}
