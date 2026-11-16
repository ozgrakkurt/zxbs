use std::path::{Path, PathBuf};

use crate::cache::CacheState;
use crate::cache::write_wal_file as write_cache_wal_file;
use crate::common::check_name;
use crate::config::{Config, Subcommand};
use crate::hash::{Hash, HashMap, HashSet, hash_bytes, hash_ordered, mix_hashes};
use crate::interner::{Interner, Str};
use crate::thread_pool::{JoinSet, ThreadPool};

pub fn make_module_name(root_name: &str, part_name: &str) -> String {
    if root_name == part_name {
        root_name.to_owned()
    } else {
        format!("{}:{}", root_name, part_name)
    }
}

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
            Subcommand::Clean => {
                // this subcommand should be handled in main.rs so should never come here.
                unreachable!();
            }
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
            "Subcommand argument has entry `{}:{}`. But the entry `{}` doesn't have a {} named `{}`",
            entry_name, part_name, entry_name, kind_name, part_name
        );
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

pub fn parse_deps_file(path: &Path, contents: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut dedup = HashSet::default();
    for line in contents.trim().split('\n') {
        let line = line.trim();
        if !line.is_empty() {
            check_name(line, &format!("Parse deps file at: `{}`", path.display()));
            assert!(
                dedup.insert(line),
                "Deps file at `{}` contains a duplicate dependency on `{}`",
                path.display(),
                line
            );
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
    .unwrap_or_else(|e| {
        panic!(
            "failed to parse UTF8 from file at: {}. Error={}",
            path.display(),
            e
        )
    });
    parse_deps_file(&path, contents.as_str())
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

    check_name(name, &format!("Parse file name of: {}", path.display()));

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
            panic!(
                "Expected the file at {} to be a regular file but it isn't.",
                file_path.display()
            );
        }
        let name = parse_file_name(
            &file_path,
            expected_extension,
            file.file_name().to_str().unwrap(),
        )
        .to_owned();
        check_name(
            &name,
            &format!("Name of file at: `{}`", file_path.display()),
        );
        names.push(name);
    }
    names
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
    deps: CppmDeps,
}

fn check_provides(
    interner: &Interner,
    entry_names: &[Str],
    mod_list: &[Vec<Str>],
    target: &ScanTarget,
    provides: &[String],
) {
    match target.kind {
        ScanTargetKind::Mod => {
            let entry_name = interner.get(entry_names[target.entry_index]);
            let part_name = interner.get(mod_list[target.entry_index][target.part_index]);
            let expected_provides = make_module_name(entry_name, part_name);
            if provides.len() != 1 || provides[0].as_str() != expected_provides.as_str() {
                panic!(
                    "expected `{}` to provide `{}`, but it provides: `{:?}`",
                    target.path, expected_provides, provides
                );
            }
        }
        _ => {
            if !provides.is_empty() {
                panic!(
                    "Didn't expect `{}` to provide any module interface. But it provides: `{:?}`",
                    target.path, provides
                );
            }
        }
    }
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
    target_kind: ScanTargetKind,
) {
    for (part_index, part_name) in parts.iter().enumerate() {
        let path = entry_path
            .join(kind_name)
            .join(format!("{}.{}", part_name, extension));
        let path_idx = interner.insert(path.to_str().unwrap());
        let path = interner.get(path_idx);

        let target = ScanTarget {
            kind: target_kind,
            entry_index,
            part_index,
            path,
        };

        hashes_join_set.spawn(thread_pool, move || {
            let data = std::fs::read(target.path).unwrap();
            let basic_hash = hash_ordered(&[target.path.as_bytes(), data.as_slice()]);
            (target, basic_hash)
        });
    }
}

fn check_cyclic_dependencies_between_entries(
    interner: &Interner,
    entry_names: &[Str],
    entry_name_map: &HashMap<Str, u32>,
    deps_list: &[Vec<Str>],
) {
    let num_entries = entry_names.len();

    assert_eq!(num_entries, deps_list.len());

    // Resolve the dependency names of each entry to indices first so the dfs
    // below can index straight into the entry list, just like the module parts
    // check indexes into its dense matrix.
    let deps_indices = deps_list
        .iter()
        .enumerate()
        .map(|(entry_index, deps)| {
            deps.iter()
                .map(|&dep_name| {
                    let dep_idx = entry_name_map.get(&dep_name).unwrap_or_else(|| {
                        panic!(
                            "Entry `{}` depends on `{}` which is not an entry.",
                            interner.get(entry_names[entry_index]),
                            interner.get(dep_name),
                        )
                    });
                    usize::try_from(*dep_idx).unwrap()
                })
                .collect::<Vec<usize>>()
        })
        .collect::<Vec<Vec<usize>>>();

    if num_entries == 0 {
        return;
    }

    let mut visited = vec![false; num_entries];
    let mut current_stack = Vec::new();
    let mut is_in_current_stack = vec![false; num_entries];

    struct State {
        entry_idx: usize,
        dep_pos: usize,
    }

    for start_entry_idx in 0..num_entries {
        if visited[start_entry_idx] {
            continue;
        }

        // We use this for doing a depth first search without recursion.
        // Record current entry idx and current dep_pos we are at in the conceptual loop.
        // Think as if these are the values of the local variables at the point of recursion call.
        //
        // This is the standard algorithm you would find anywhere for cycle detection just modified to
        // run without recursion.
        let mut recursion = vec![State {
            entry_idx: start_entry_idx,
            dep_pos: 0,
        }];

        while let Some(State { entry_idx, dep_pos }) = recursion.pop() {
            // Before the dfs loop, as in we just arrived at this layer.
            if dep_pos == 0 {
                assert!(!visited[entry_idx]);
                assert!(!is_in_current_stack[entry_idx]);

                visited[entry_idx] = true;
                is_in_current_stack[entry_idx] = true;
                current_stack.push(entry_idx);
            }

            let mut dep_pos = dep_pos;

            while dep_pos < deps_indices[entry_idx].len() {
                let dep_entry_idx = deps_indices[entry_idx][dep_pos];

                if is_in_current_stack[dep_entry_idx] {
                    // We found a cycle, print the error and exit.

                    let mut err_msg =
                        String::from("Found a dependency cycle between entries. Cycle is: ");

                    let cycle_start_pos = current_stack
                        .iter()
                        .position(|&x| x == dep_entry_idx)
                        .unwrap();

                    for &p in current_stack[cycle_start_pos..].iter() {
                        err_msg.push_str(interner.get(entry_names[p]));
                        err_msg.push_str(" -> ");
                    }

                    err_msg.push_str(interner.get(entry_names[dep_entry_idx]));

                    panic!("{}", err_msg);
                }

                // Optimization: don't go into the dep entry if the dep is already visited.
                if !visited[dep_entry_idx] {
                    // these next two pushes equate to a recursive call to dfs search

                    // resume at the next dep of the loop when we come back
                    recursion.push(State {
                        entry_idx,
                        dep_pos: dep_pos + 1,
                    });
                    // go to the lower layer
                    recursion.push(State {
                        entry_idx: dep_entry_idx,
                        dep_pos: 0,
                    });

                    break;
                }

                dep_pos += 1;
            }

            if dep_pos == deps_indices[entry_idx].len() {
                // We finished the inner dfs loop, just remove ourselves from the current search stack which
                // means to going to the above layer in dfs search.
                is_in_current_stack[entry_idx] = false;
                assert_eq!(entry_idx, current_stack.pop().unwrap());
            }
        }
    }

    for v in visited {
        assert!(v);
    }
}

fn check_cyclic_dependencies_between_module_parts(
    interner: &Interner,
    entry_name: Str,
    mod_list: &[Str],
    mod_deps_matrix: &[bool],
) {
    let num_parts = mod_list.len();

    assert_eq!(num_parts * num_parts, mod_deps_matrix.len());

    if num_parts == 0 {
        return;
    }

    let mut visited = vec![false; num_parts];
    let mut current_stack = Vec::new();
    let mut is_in_current_stack = vec![false; num_parts];

    struct State {
        part_idx: usize,
        dep_idx: usize,
    }

    for start_part_idx in 0..num_parts {
        if visited[start_part_idx] {
            continue;
        }

        // We use this for doing a depth first search without recursion.
        // Record current part idx and current dep_idx we are at in the conceptual loop.
        // Think as if these are the values of the local variables at the point of recursion call.
        //
        // This is the standard algorithm you would find anywhere for cycle detection just modified to
        // run without recursion.
        let mut recursion = vec![State {
            part_idx: start_part_idx,
            dep_idx: 0,
        }];

        while let Some(State { part_idx, dep_idx }) = recursion.pop() {
            // Before the dfs loop, as in we just arrived at this layer.
            if dep_idx == 0 {
                assert!(!visited[part_idx]);
                assert!(!is_in_current_stack[part_idx]);

                visited[part_idx] = true;
                is_in_current_stack[part_idx] = true;
                current_stack.push(part_idx);
            }

            let mut dep_idx = dep_idx;

            while dep_idx < num_parts {
                if mod_deps_matrix[part_idx * num_parts + dep_idx] {
                    if is_in_current_stack[dep_idx] {
                        // We found a cycle, print the error and exit.

                        let mut err_msg = format!(
                            "Found a module dependency cycle inside the entry `{}`. Cycle is: ",
                            interner.get(entry_name)
                        );

                        let cycle_start_pos =
                            current_stack.iter().position(|&x| x == dep_idx).unwrap();

                        for &p in current_stack[cycle_start_pos..].iter() {
                            err_msg.push_str(interner.get(mod_list[p]));
                            err_msg.push_str(" -> ");
                        }

                        err_msg.push_str(interner.get(mod_list[dep_idx]));

                        panic!("{}", err_msg);
                    }

                    // Optimization: don't go into the dep layer if the dep is already visited.
                    if !visited[dep_idx] {
                        // these next two pushes equate to a recursive call to dfs search

                        // resume at the next iteration of the loop when we come back
                        recursion.push(State {
                            part_idx,
                            dep_idx: dep_idx + 1,
                        });
                        // go to the lower layer
                        recursion.push(State {
                            part_idx: dep_idx,
                            dep_idx: 0,
                        });

                        break;
                    }
                }

                dep_idx += 1;
            }

            if dep_idx == num_parts {
                // We finished the inner dfs loop, just remove ourselves from the current search stack which
                // means to going to the above layer in dfs search.
                is_in_current_stack[part_idx] = false;
                assert_eq!(part_idx, current_stack.pop().unwrap());
            }
        }
    }

    for v in visited {
        assert!(v);
    }
}

// make sure that we only have the expected content in the entry folder.
//
// - mod folder
// - lib folder
// - bin folder
// - test folder
// - deps file
fn check_entry_folder_content(path: &Path) {
    let dir = match std::fs::read_dir(path) {
        Ok(v) => v,
        Err(e) => {
            panic!("Failed to read_dir on '{}'. Error: {}", path.display(), e);
        }
    };

    let mut found_mod = false;
    let mut found_lib = false;
    let mut found_bin = false;
    let mut found_test = false;
    let mut found_deps = false;

    for entry in dir {
        let entry = entry.unwrap();
        let file_path = entry.path();
        let file_name = file_path.file_name().unwrap().to_str().unwrap();
        let file_type = entry.file_type().unwrap();

        if file_type.is_dir() {
            if file_name == "mod" {
                found_mod = true;
            } else if file_name == "lib" {
                found_lib = true;
            } else if file_name == "bin" {
                found_bin = true;
            } else if file_name == "test" {
                found_test = true;
            } else {
                panic!(
                    "Found unexpected directory `{}` in entry directory at: `{}`",
                    file_name,
                    path.display()
                );
            }
        } else if file_type.is_file() {
            if file_name != "deps" {
                panic!(
                    "Found unexpected file `{}` in entry directory at: `{}`",
                    file_name,
                    path.display()
                );
            } else {
                found_deps = true;
            }
        } else {
            panic!(
                "Found unexpected type of file `{}` in entry directory at: `{}`. Only expected folders and regular files.",
                file_name,
                path.display()
            );
        }
    }

    assert!(
        found_mod,
        "Expected the entry folder at `{}` to contain a `mod` folder but it doesn't.",
        path.display()
    );
    assert!(
        found_lib,
        "Expected the entry folder at `{}` to contain a `lib` folder but it doesn't.",
        path.display()
    );
    assert!(
        found_bin,
        "Expected the entry folder at `{}` to contain a `bin` folder but it doesn't.",
        path.display()
    );
    assert!(
        found_test,
        "Expected the entry folder at `{}` to contain a `test` folder but it doesn't.",
        path.display()
    );
    assert!(
        found_deps,
        "Expected the entry folder at `{}` to contain a `deps` file but it doesn't.",
        path.display()
    );
}

pub fn scan(
    config: &'static Config,
    interner: &mut Interner,
    thread_pool: &ThreadPool,
    base_hash: Hash,
    cache_state: &mut CacheState,
    timestamp: u64,
) -> BuildInfo {
    let dir = std::fs::read_dir(config.src_path).unwrap();

    // First list the source directory to find every entry.
    let mut entry_names: Vec<Str> = Vec::new();
    let mut entry_name_map: HashMap<Str, u32> = HashMap::default();
    for (entry_index, entry) in dir.enumerate() {
        let entry = entry.unwrap();
        let path = entry.path();

        if !entry.file_type().unwrap().is_dir() {
            panic!(
                "Expected source directory to only contain directories but found something that isn't a directory at `{}`",
                path.display()
            );
        }

        let name = path.file_name().unwrap().to_str().unwrap();
        check_name(name, "Parse entry name");

        let name = interner.insert(name);

        entry_names.push(name);
        entry_name_map.insert(name, u32::try_from(entry_index).unwrap());
    }
    let entry_names = entry_names;
    let entry_name_map = entry_name_map;

    // List each entry's mod/test/bin/lib folders and read it's deps
    // file.
    let mut listing_join_set: JoinSet<ListingResult> = JoinSet::new();

    // just for checking the folder contents of each entry.
    let mut check_entry_folder_join_set: JoinSet<()> = JoinSet::new();

    for (entry_index, &entry_name) in entry_names.iter().enumerate() {
        let entry_path = config.src_path.join(interner.get(entry_name));

        let p = entry_path.join("deps");
        listing_join_set.spawn(thread_pool, move || ListingResult {
            kind: ListingResultKind::Deps,
            entry_index,
            parts: read_deps_file(p),
        });

        let p = entry_path.join("mod");
        listing_join_set.spawn(thread_pool, move || ListingResult {
            kind: ListingResultKind::Mod,
            entry_index,
            parts: scan_subfolder("cppm", &p),
        });

        let p = entry_path.join("lib");
        listing_join_set.spawn(thread_pool, move || ListingResult {
            kind: ListingResultKind::Lib,
            entry_index,
            parts: scan_subfolder("cc", &p),
        });

        let p = entry_path.join("test");
        listing_join_set.spawn(thread_pool, move || ListingResult {
            kind: ListingResultKind::Test,
            entry_index,
            parts: scan_subfolder("cc", &p),
        });

        let p = entry_path.join("bin");
        listing_join_set.spawn(thread_pool, move || ListingResult {
            kind: ListingResultKind::Bin,
            entry_index,
            parts: scan_subfolder("cc", &p),
        });

        check_entry_folder_join_set.spawn(thread_pool, move || {
            check_entry_folder_content(&entry_path);
        });
    }

    let mut deps_list: Vec<Vec<Str>> = vec![Vec::new(); entry_names.len()];
    let mut mod_list: Vec<Vec<Str>> = vec![Vec::new(); entry_names.len()];
    let mut lib_list: Vec<Vec<Str>> = vec![Vec::new(); entry_names.len()];
    let mut test_list: Vec<Vec<Str>> = vec![Vec::new(); entry_names.len()];
    let mut bin_list: Vec<Vec<Str>> = vec![Vec::new(); entry_names.len()];

    // We will populate targets and start jobs on the hashes_join_set as the results
    // are coming from the listing_join_set.
    //
    // We could defer spawning into hashes_join_set until later but doing this means we are
    // pipelining two tasks as much as possible. We will use the same pattern in other parts
    // of this function.
    let mut hashes_join_set: JoinSet<(ScanTarget, Hash)> = JoinSet::new();

    while let Some(result) = listing_join_set.join_next() {
        let entry_name_idx = entry_names[result.entry_index];
        let entry_name = interner.get(entry_name_idx);
        let entry_path = config.src_path.join(entry_name);
        match result.kind {
            ListingResultKind::Mod => {
                spawn_hash_scan_targets(
                    result.entry_index,
                    &entry_path,
                    &result.parts,
                    interner,
                    thread_pool,
                    &mut hashes_join_set,
                    "mod",
                    "cppm",
                    ScanTargetKind::Mod,
                );

                mod_list[result.entry_index] = result
                    .parts
                    .iter()
                    .map(|name| interner.insert(name.as_str()))
                    .collect::<Vec<Str>>();
            }
            ListingResultKind::Lib => {
                spawn_hash_scan_targets(
                    result.entry_index,
                    &entry_path,
                    &result.parts,
                    interner,
                    thread_pool,
                    &mut hashes_join_set,
                    "lib",
                    "cc",
                    ScanTargetKind::Lib,
                );

                lib_list[result.entry_index] = result
                    .parts
                    .iter()
                    .map(|name| interner.insert(name.as_str()))
                    .collect::<Vec<Str>>();
            }
            ListingResultKind::Test => {
                spawn_hash_scan_targets(
                    result.entry_index,
                    &entry_path,
                    &result.parts,
                    interner,
                    thread_pool,
                    &mut hashes_join_set,
                    "test",
                    "cc",
                    ScanTargetKind::Test,
                );

                test_list[result.entry_index] = result
                    .parts
                    .iter()
                    .map(|name| interner.insert(name.as_str()))
                    .collect::<Vec<Str>>();
            }
            ListingResultKind::Bin => {
                spawn_hash_scan_targets(
                    result.entry_index,
                    &entry_path,
                    &result.parts,
                    interner,
                    thread_pool,
                    &mut hashes_join_set,
                    "bin",
                    "cc",
                    ScanTargetKind::Bin,
                );

                bin_list[result.entry_index] = result
                    .parts
                    .iter()
                    .map(|name| interner.insert(name.as_str()))
                    .collect::<Vec<Str>>();
            }
            ListingResultKind::Deps => {
                // no spawn_hash_scan_targets for deps obviously, they are just a list of entry names.

                // check that the entry doesn't depend on itself (:
                //
                // We do a real cyclic dependency check for entries later in this function but doing this check here still is nice to catch
                // simple mistakes early.
                let mut deps = Vec::with_capacity(result.parts.len());
                for name in result.parts.iter() {
                    let name_idx = interner.insert(name.as_str());
                    assert!(
                        name_idx != entry_name_idx,
                        "Entry `{}` depends on itself. It lists it's own name in the deps file.",
                        entry_name
                    );
                    deps.push(name_idx);
                }

                deps_list[result.entry_index] = deps;
            }
        }
    }

    check_entry_folder_join_set.join_all();

    for (entry_index, modules) in mod_list.iter().enumerate() {
        let entry_name = entry_names[entry_index];
        assert!(
            modules.contains(&entry_name),
            "root module of entry {} not found",
            interner.get(entry_name),
        );
    }

    check_cyclic_dependencies_between_entries(interner, &entry_names, &entry_name_map, &deps_list);

    // make these immutable
    let deps_list = deps_list;
    let mod_list = mod_list;
    let lib_list = lib_list;
    let test_list = test_list;
    let bin_list = bin_list;

    // Pipeline finished hashing tasks into the clang_scan_deps tasks.
    let mut scan_deps_join_set: JoinSet<ScanResult> = JoinSet::new();

    let mut basic_hashes: HashMap<Str, Hash> = HashMap::default();

    while let Some((scan_target, basic_hash)) = hashes_join_set.join_next() {
        assert!(
            basic_hashes
                .insert(interner.insert(scan_target.path), basic_hash)
                .is_none()
        );

        // We use this deep hash for the clang-scan-deps result of this file.
        let scan_deps_hash = mix_hashes(
            mix_hashes(base_hash, basic_hash),
            // Mix a deterministic marker in so the deep hash doesn't collide with
            // some other deep hash.
            hash_bytes(b"clang-scan-deps-marker"),
        );

        let cache_path = config.cache_path.join(format!("{:x}", scan_deps_hash));

        if let Some(target_hash) = cache_state.item(scan_deps_hash, timestamp) {
            // Just load from cache
            scan_deps_join_set.spawn(thread_pool, move || {
                let raw = std::fs::read(&cache_path).unwrap();
                assert_eq!(target_hash, hash_bytes(&raw));
                // Could store the parsed thing and avoid parsing json but
                // parsing json isn't the problem here.
                //
                // Running clang-scan-deps takes an insane amount of time so we
                // mainly want to avoid that.
                let deps = parse_p1689(&raw);

                ScanResult {
                    target: scan_target,
                    deps,
                }
            });
        } else {
            // Do actual scan and store to cache

            let path = scan_target.path;

            scan_deps_join_set.spawn(thread_pool, move || {
                let raw = run_scan_deps(config, Path::new(path));

                let output_hash = hash_bytes(&raw);

                let tmp_path = format!("{}.tmp", cache_path.display());
                std::fs::write(&tmp_path, &raw).unwrap();
                std::fs::rename(&tmp_path, &cache_path).unwrap();

                let output_size = u32::try_from(raw.len()).unwrap();

                write_cache_wal_file(config.cache_path, scan_deps_hash, output_hash, output_size);

                let deps = parse_p1689(&raw);

                ScanResult {
                    target: scan_target,
                    deps,
                }
            });
        }
    }
    let basic_hashes = basic_hashes;

    let mut mod_dep_matrix: Vec<Vec<bool>> = Vec::with_capacity(mod_list.len());
    // Initialize beforehand so we can just index into this thing later.
    //
    // Initializing at the time we receive the values would be more annoying.
    for parts in mod_list.iter() {
        mod_dep_matrix.push(vec![false; parts.len() * parts.len()]);
    }

    while let Some(scan_result) = scan_deps_join_set.join_next() {
        let ScanResult { target, deps } = scan_result;

        // Make sure the module exports of the file is as expected.
        check_provides(interner, &entry_names, &mod_list, &target, &deps.provides);

        let entry_mod_parts = &mod_list[target.entry_index];
        let num_parts = entry_mod_parts.len();

        let entry_name_idx = entry_names[target.entry_index];
        let entry_name = interner.get(entry_name_idx);

        let entry_deps = &deps_list[target.entry_index];

        match target.kind {
            ScanTargetKind::Mod => {
                let entry_mod_parts = &mod_list[target.entry_index];
                let part_name_idx: Str = entry_mod_parts[target.part_index];
                let part_name = interner.get(part_name_idx);

                // Construct the module dependency matrix
                let matrix = &mut mod_dep_matrix[target.entry_index];
                for dep_name in deps.deps.iter() {
                    let dep_name_idx = interner.insert(dep_name.as_str());
                    if dep_name.contains(":") {
                        let parts = dep_name.split(':').collect::<Vec<&str>>();
                        if parts.len() != 2 {
                            panic!(
                                "The submodule `{}:{}` has an invalid dependency name parsed by clang-scan-deps: `{}`",
                                entry_name, part_name, dep_name
                            );
                        }

                        if parts[0] != entry_name {
                            panic!(
                                "The submodule `{}:{}` has an invalid dependency on `{}`. It is only allowed to depend on other submodules in the same entry. Depend on the root module of another entry to use a submodule of that entry.",
                                entry_name, part_name, dep_name
                            );
                        }

                        let dep_sub_name_idx = interner.insert(parts[1]);
                        let dep_sub_name = interner.get(dep_sub_name_idx);

                        if dep_sub_name_idx == part_name_idx {
                            panic!(
                                "The submodule `{}:{}` depends on itself.",
                                entry_name, part_name
                            );
                        }

                        let dep_index = match entry_mod_parts
                            .iter()
                            .position(|&x| x == dep_sub_name_idx)
                        {
                            None => {
                                panic!(
                                    "The submodule `{}:{}` depends on `{}:{}` but `{}` doesn't have a submodule named `{}`",
                                    entry_name,
                                    part_name,
                                    entry_name,
                                    dep_sub_name,
                                    entry_name,
                                    dep_sub_name,
                                );
                            }
                            Some(idx) => idx,
                        };

                        matrix[num_parts * target.part_index + dep_index] = true;
                    } else {
                        // has to be an dependended-on entry name, can't be anything else.

                        // can't depend on own root
                        if dep_name_idx == entry_name_idx {
                            panic!(
                                "The submodule `{}:{}` depends on it's own root module. This is not allowed as the root is supposed to depend on all the submodules.",
                                entry_name, part_name
                            );
                        }

                        // not a dependency
                        if !entry_deps.contains(&dep_name_idx) {
                            panic!(
                                "The submodule `{}:{}` depends on the root module of another entry `{}` but this entry isn't listed in dependencies of `{}`",
                                entry_name, part_name, dep_name, entry_name
                            );
                        }
                    }
                }
            }
            _ => {
                for dep_name in deps.deps.iter() {
                    let dep_name_idx = interner.insert(dep_name.as_str());

                    if dep_name_idx != entry_name_idx && !entry_deps.contains(&dep_name_idx) {
                        panic!(
                            "The source file at `{}` depends on `{}`, this is invalid because source files are only allowed to depend on root module of their own entry or other entries that are listed in the deps file of their own entry.",
                            target.path, dep_name
                        );
                    }
                }
            }
        }
    }
    let mod_dep_matrix = mod_dep_matrix;

    for (entry_index, matrix) in mod_dep_matrix.iter().enumerate() {
        check_cyclic_dependencies_between_module_parts(
            interner,
            entry_names[entry_index],
            &mod_list[entry_index],
            matrix,
        );

        // Check that the root module depends on all of the submodules.

        let entry_name_idx = entry_names[entry_index];
        let entry_name = interner.get(entry_name_idx);
        let mod_parts = &mod_list[entry_index];

        let root_pos = mod_parts.iter().position(|&x| x == entry_name_idx).unwrap();

        for i in 0..mod_parts.len() {
            if i != root_pos && !matrix[root_pos * mod_parts.len() + i] {
                panic!(
                    "Root module of `{}` does NOT have a dependency on the submodule `{}:{}`. Root modules are expected to depend on all submodules of the entry.",
                    entry_name,
                    entry_name,
                    interner.get(mod_parts[i]),
                );
            }
        }
    }

    BuildInfo {
        name: entry_names,
        map: entry_name_map,
        module: mod_list,
        mod_dep_matrix,
        lib: lib_list,
        bin: bin_list,
        test: test_list,
        deps: deps_list,
        basic_hash: basic_hashes,
    }
}
