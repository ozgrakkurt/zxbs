use std::path::{Path, PathBuf};

use crate::check::check_name;
use crate::config::{Config, Subcommand};
use crate::hash::{Hash, HashMap, hash_bytes, hash_ordered, mix_hashes};
use crate::interner::{Interner, Str};
use crate::thread_pool::{JoinSet, ThreadPool};
use crate::cache::CacheState;

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
    })).unwrap_or_else(|e| {
        panic!(
            "failed to parse UTF8 from file at: {}. Error={}",
            path.display(),
            e
        )
    });
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
        let file_path = file.path();
        if !file.file_type().unwrap().is_file() {
            panic!("Expected the file at {} to be a regular file but it isn't.", file_path.display());
        }
        let name = parse_file_name(
            &file_path,
            expected_extension,
            file.file_name().to_str().unwrap(),
        )
        .to_owned();
        check_name(&name, "Name of file at: `{}`", file_path.display());
        names.push(name);
    }
    names
}

struct ObjectFolderInfo {
    parts: Vec<String>,
    deps: Vec<Vec<String>>,
}

fn check_object_file_dependencies(
    entry_name: &str,
    self_module_parts: &[String],
    ext_deps: &[String],
    info: &ObjectFolderInfo,
) {
    assert_eq!(info.parts.len(), info.deps.len());
    for i in 0..info.parts.len() {
        for dep in info.deps[i].iter() {
            let part_name = info.parts[i];
            
            if dep.contains(":") {
                panic!("Invalid dependency `{}` in `{}:{}`. Implementation (.cc) files are only allowed to depend on root modules.", dep, entry_name, part_name);
            }

            if dep != entry_name && !ext_deps.contains(dep) {
                panic!(
                    "Invalid dependency '{}' in '{}:{}'. It can only depend of the root module of it's entry `{}` and the root modules of external dependencies: {:?}",
                    dep, entry_name, part_name, entry_name, ext_deps
                );
            }            
        }
    }
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
                    "expected `{}` to provide `{}`, but it provides: `{:?}`",
                    path, expected_provides, cppm_deps.provides
                );
            }
        }
        _ => {
            if !cppm_deps.provides.is_empty() {
                panic!(
                    "Didn't expect `{}` to provide any module interface. But it provides: `{:?}`",
                    path, cppm_deps.provides
                );
            }
        }
    }

    cppm_deps.deps
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

#[derive(Clone, Copy)]
enum ListingResultKind {
    Deps,
    Mod,
    Lib,
    Test,
    Bin,
}

struct ListingResult {
    kind: ListingResultKind,
    entry_index: usize,
    parts: Vec<String>,
}

#[derive(Clone, Copy)]
enum ScanTargetKind {
    Mod,
    Lib,
    Test,
    Bin,
}

// A single source file that needs to be scanned with clang-scan-deps (or read
// from the cache) and hashed.
#[derive(Clone, Copy)]
struct ScanTarget {
    kind: ScanTargetKind,
    entry_index: usize,
    part_index: usize,
    path: &'static str,
}

struct ScanResult {
    target: ScanTarget,
    deps: Vec<String>,
    basic_hash: Hash,
}

fn spawn_hash_scan_targets(
    entry_index: usize,
    entry_path: &Path,
    parts: &[String],
    interner: &mut Interner,
    thread_pool: &ThreadPool,
    hashes_join_set: &mut JoinSet<(ScanTarget, Hash)>,
    kind_name: &str,
    extension: &str,
) {
    for (part_index, part_name) in parts.iter().enumerate() {
        let path = entry_path.join(kind_name).join(format!("{}.{}", part_name, extension));
        let path_idx = interner.insert(path.to_str().unwrap());
        let path = interner.get(path_idx);

        let target = ScanTarget {
           kind: ScanTargetKind::Mod,
           entry_index: result.entry_index,
           part_index,
           path,
        };

        hashes_join_set.spawn(thread_pool, move || {
            let data = std::fs::read(scan_target.path).unwrap();
            let basic_hash = hash_ordered(&[scan_target.path.as_bytes(), data.as_slice()]);
            (scan_target, basic_hash)
        });
    }
}

pub fn scan(
    config: &'static Config,
    interner: &mut Interner,
    thread_pool: &ThreadPool,
    base_hash: Hash,
    cache_state: &mut CacheState,
) -> BuildInfo {
    let dir = std::fs::read_dir(config.src_path).unwrap();

    // First list the source directory to find every entry.
    let mut entry_names: Vec<Str> = Vec::new();
    for entry in dir {
        let entry = entry.unwrap();
        let path = entry.path();

        if !entry.file_type().unwrap().is_dir() {
            panic!("Expected source directory to only contain directories but found something that isn't a directory at `{}`", path.display());
        }

        let name = path.file_name().unwrap().to_str().unwrap();
        check_name(name);

        let name = interner.insert(name);

        entry_names.push(name);
    }
    let entry_names = entry_names;

    // List each entry's mod/test/bin/lib folders and read it's deps
    // file.
    let mut listing_join_set: JoinSet<ListingResult>  = JoinSet::new();
    for (entry_index, entry_name) in entries_names.iter().enumerate() {
        let entry_path = config.src_path.join(interner.get(entry_name));
        
        let p = entry_path.join("deps");
        listing_join_set.spawn(thread_pool, move || ListingResult {
            kind: ListingResultKind::Deps,
            entry_index,
            parts: read_deps_file(p),
        });

        let p = entry_path.join("mod");
        listing_join_set.spawn(thread_pool, move || ListingResult::Mod {
            entry,
            parts: scan_subfolder("cppm", &p),
        });

        let p = entry_path.join("lib");
        listing_join_set.spawn(thread_pool, move || ListingResult::Lib {
            entry,
            parts: scan_subfolder("cc", &p),
        });

        let p = entry_path.join("test");
        listing_join_set.spawn(thread_pool, move || ListingResult::Test {
            entry,
            parts: scan_subfolder("cc", &p),
        });

        let p = entry_path.join("bin");
        listing_join_set.spawn(thread_pool, move || ListingResult::Bin {
            entry,
            parts: scan_subfolder("cc", &p),
        });
    }

    let mut deps_list = vec![Vec::new(); entries.len()];
    let mut mod_list = vec![Vec::new(); entries.len()];
    let mut lib_list = vec![Vec::new(); entries.len()];
    let mut test_list = vec![Vec::new(); entries.len()];
    let mut bin_list = vec![Vec::new(); entries.len()];

    // We will populate targets and start jobs on the hashes_join_set as the results
    // are coming from the listing_join_set.
    //
    // We could defer spawning into hashes_join_set until later but doing this means we are
    // pipelining two tasks as much as possible. We will use the same pattern in other parts
    // of this function.
    let mut hashes_join_set: JoinSet<(ScanTarget, Hash)> = JoinSet::new();

    while let Some(result) = listing_join_set.join_next() {
        let entry_path: &Path = &entries[result.entry_index].0;
        match result.kind {
            ListingResultKind::Mod => {
                spawn_hash_scan_targets(
                    result.entry_index,
                    entry_path,
                    &result.parts,
                    interner,
                    thread_pool,
                    &mut hashes_join_set,
                    "mod",
                    "cppm",
                );
                
                mod_list.push(result.parts);
            }
            ListingResultKind::Lib => {
                spawn_hash_scan_targets(
                    result.entry_index,
                    entry_path,
                    &result.parts,
                    interner,
                    thread_pool,
                    &mut hashes_join_set,
                    "lib",
                    "cc",
                );

                lib_list.push(result.parts);
            }
            ListingResultKind::Test => {
                spawn_hash_scan_targets(
                    result.entry_index,
                    entry_path,
                    &result.parts,
                    interner,
                    thread_pool,
                    &mut hashes_join_set,
                    "test",
                    "cc",
                );

                test_list.push(result.parts);
            }
            ListingResultKind::Bin => {
                spawn_hash_scan_targets(
                    result.entry_index,
                    entry_path,
                    &result.parts,
                    interner,
                    thread_pool,
                    &mut hashes_join_set,
                    "bin",
                    "cc",
                );

                bin_list.push(result.parts);
            }
            ListingResultKind::Deps => {
                // no scan targets for deps obviously, they are just a list of entry names.
                // 
                deps_list.push(result.parts);
            }
        }
    }

    for (entry_index, modules) in mod_list.iter().enumerate() {
        let entry_name = &entries[entry_index].1;
        assert!(
            modules.contains(entry_name),
            "root module of entry {} not found",
            name,
        );
    }

    assert_eq!(entries.len(), deps_list.len());

    assert_eq!(deps_list.len(), mod_list.len());
    assert_eq!(deps_list.len(), lib_list.len());
    assert_eq!(deps_list.len(), test_list.len());
    assert_eq!(deps_list.len(), bin_list.len());

    // make these immutable
    let deps_list = deps_list;
    let mod_list = mod_list;
    let lib_list = lib_list;
    let test_list = test_list;
    let bin_list = bin_list;

    // Pipeline finished hashing tasks into the clang_scan_deps tasks.
    let mut scan_deps_join_set: JoinSet<ScanResult> = JoinSet::new();

    while let Some((scan_target, basic_hash)) = hashes_join_set.join_next() {
        // We use this deep hash for the clang-scan-deps result of this file.
        let scan_deps_hash = mix_hashes(
            mix_hashes(base_hash, basic_hash),
            // Mix a deterministic marker in so the deep hash doesn't collide with
            // some other deep hash.
            hash_bytes(b"clang-scan-deps-marker"),
        );

        let entry_name = &entries[scan_target.entry_index].1;
        let cache_path = config.cache_path.join(format!("{:x}", scan_deps_hash));

        if let Some(target_hash) = cache_state.item(scan_deps_hash) {
            // Just load from cache
            scan_deps_join_set.spawn(thread_pool, move || {

                ScanResult {
                    target: scan_target,
                    deps,
                    basic_hash,
                }
            });
        } else {
            // Do actual scan and store to cache

            let file_path = config.src_path.join(entry_name);
            
            scan_deps_join_set.spawn(thread_pool, move || {
                let raw = run_scan_deps(config, Path::new(&path));
                std::fs::write(&cache_path, &raw).unwrap();
                parse_p1689(&raw)
                
                ScanResult {
                    target: scan_target,
                    deps,
                    basic_hash,
                }
            });
        }
    }

    while let Some(scan_result) = scan_deps_join_set.join_next() {
        
    }

    // Second phase: hash and scan every source file in parallel. Each task
    // reads the file and hashes it, then either loads the clang-scan-deps
    // result from the cache or runs clang-scan-deps and stores the result.
    let cache_files_path = config.cache_path.to_str().unwrap();
    let mut join_set: JoinSet<ScanResult> = JoinSet::new();
    for target in &targets {
        let entry = target.entry;
        let kind = target.kind;
        let part = target.part;
        let path = target.path.clone();
        let entry_name = listings[entry].name.clone();
        let part_name = listing_part_name(&listings[entry], kind, part).to_owned();
        join_set.spawn(thread_pool, move || {

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

    let mut info = BuildInfo::default();

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
