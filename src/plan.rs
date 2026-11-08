use std::collections::HashSet;

use xxhash_rust::xxh3::Xxh3 as HashBuilder;

use crate::build::Task;
use crate::cache::CacheState;
use crate::command::{
    ArchiveLib, CacheLoad, CacheStore, Command, CompileModule, CompileToObject, Format, LinkBinary,
    RunBinary, RunTest,
};
use crate::config::{Config, Subcommand};
use crate::interner::{Interner, Str};
use crate::scan::BuildInfo;

pub fn make_module_artifact_name(root_name: &str, part_name: &str, extension: &str) -> String {
    if root_name == part_name {
        format!("{}.{}", root_name, extension)
    } else {
        format!("{}-{}.{}", root_name, part_name, extension)
    }
}

// checks if given test should run according to the subcommand.
pub fn should_run_test(subcommand: &Subcommand, entry: &str, part: &str) -> bool {
    match subcommand {
        Subcommand::TestAll => true,
        Subcommand::Test {
            entry_name,
            test_name,
        } => entry == entry_name && part == test_name,
        _ => false,
    }
}

// checks if given binary should run according to the subcommand.
pub fn should_run_binary(
    subcommand: &Subcommand,
    entry: &str,
    part: &str,
) -> Option<&'static [&'static str]> {
    match subcommand {
        Subcommand::RunBinary {
            entry_name,
            bin_name,
            args,
        } => {
            if entry == entry_name && part == bin_name {
                Some(args)
            } else {
                None
            }
        }
        _ => None,
    }
}

struct Params<'a> {
    config: &'a Config,
    subcommand: &'a Subcommand,
    interner: &'a mut Interner,
    info: &'a BuildInfo,
    cache_state: &'a mut CacheState,
    entry_idx: usize,
    deep_hashes: &'a DeepHashes,
    tasks: &'a mut Vec<Task>,
}

fn plan_module_build(params: Params) {
    let mod_parts = params.info.module[entry_index].as_slice();
    let n_mod_parts = params.mod_parts.len();

    let entry_deps = params.info.deps[entry_index].as_slice();

    let mut ext_pcm_paths: Vec<Str> = Vec::with_capacity(entry_deps.len());
    let mod_build_path = config.build_path.join("mod");
    std::fs::create_dir_all(&mod_build_path).unwrap();
    for &dep_name in entry_deps.iter() {
        let dep_name = interner.get(dep_name);
        let dep_pcm_path = mod_build_path.join(format!("{}.pcm", dep_name));
        let dep_pcm_path = interner.insert(dep_pcm_path.to_str().unwrap());
        ext_pcm_paths.push(dep_pcm_path);
    }

    let entry_name = interner.get(info.name[entry_index]);

    let mut local_pcm_paths: Vec<Str> = Vec::with_capacity(n_mod_parts);
    let mut local_object_paths: Vec<Str> = Vec::with_capacity(n_mod_parts);
    for &part_name in mod_parts.iter() {
        let part_name = interner.get(part_name);
        let part_pcm_path =
            mod_build_path.join(make_module_artifact_name(entry_name, part_name, "pcm"));
        let part_object_path =
            mod_build_path.join(make_module_artifact_name(entry_name, part_name, "o"));
        let part_pcm_path = interner.insert(part_pcm_path.to_str().unwrap());
        let part_object_path = interner.insert(part_object_path.to_str().unwrap());
        local_pcm_paths.push(part_pcm_path);
        local_object_paths.push(part_object_path);
    }

    let mod_src_path = config
        .src_path
        .join(interner.get(info.name[entry_index]))
        .join("mod");

    for (part_idx, &part) in mod_parts.iter().enumerate() {
        let mut input_indices = ext_pcm_paths.clone();
        let mod_dep_matrix_offset = n_mod_parts * part_idx;
        for index in info.mod_dep_matrix[entry_index]
            [mod_dep_matrix_offset..mod_dep_matrix_offset + n_mod_parts]
            .iter()
            .enumerate()
            .filter(|(_, is_dep)| **is_dep)
            .map(|(idx, _)| idx)
        {
            input_indices.push(local_pcm_paths[index]);
        }

        let output_indices = vec![local_pcm_paths[part_idx], local_object_paths[part_idx]];

        let cppm_path = mod_src_path.join(format!("{}.cppm", interner.get(part)));
        let cppm_path = interner.insert(cppm_path.to_str().unwrap());
        let cppm_path = interner.get(cppm_path);

        tasks.push(Task {
            entry_name: info.name[entry_index],
            part_name: part,
            inputs: input_indices,
            outputs: output_indices,
            command: Some(Command::CompileModule(CompileModule {
                cxx: config.cxx,
                cxxflags: config.cxxflags,
                input: cppm_path,
                object_output: interner.get(local_object_paths[part_idx]),
                pcm_output: interner.get(local_pcm_paths[part_idx]),
            })),
            entry_index,
        });
    }
}

fn plan_lib_build(
    config: &Config,
    interner: &mut Interner,
    tasks: &mut Vec<Task>,
    info: &BuildInfo,
    entry_index: usize,
) {
    // build object files (both .cc and .s)
    // archive into .a file

    let entry_name = interner.get(info.name[entry_index]);

    let parts = &info.lib[entry_index];
    let src_path = config.src_path.join(entry_name).join("lib");
    let build_path = config.build_path.join(entry_name).join("lib");

    std::fs::create_dir_all(&build_path).unwrap();

    let root_pcm_path = config
        .build_path
        .join("mod")
        .join(format!("{}.pcm", entry_name));
    let root_pcm_path_index = interner.insert(root_pcm_path.to_str().unwrap());

    let mod_build_path = config.build_path.join("mod");

    let mod_parts = &info.module[entry_index];

    let mut obj_path_indices: Vec<Str> = Vec::with_capacity(parts.len() + mod_parts.len());
    let mut obj_paths: Vec<&'static str> = Vec::with_capacity(parts.len() + mod_parts.len());

    for &mod_part_name_idx in mod_parts.iter() {
        let part_name = interner.get(mod_part_name_idx);
        let mod_part_obj_path =
            mod_build_path.join(make_module_artifact_name(entry_name, part_name, "o"));
        let path_idx = interner.insert(mod_part_obj_path.to_str().unwrap());
        obj_path_indices.push(path_idx);
        obj_paths.push(interner.get(path_idx));
    }

    for &part_name_idx in parts.iter() {
        let part_name = interner.get(part_name_idx);
        let cc_path = src_path.join(format!("{}.cc", part_name));
        let cc_path_index = interner.insert(cc_path.to_str().unwrap());
        let cc_path = interner.get(cc_path_index);
        let obj_path = build_path.join(format!("{}.o", part_name));
        let obj_path_index = interner.insert(obj_path.to_str().unwrap());
        let obj_path = interner.get(obj_path_index);

        obj_path_indices.push(obj_path_index);
        obj_paths.push(interner.get(obj_path_index));

        let input_indices = vec![root_pcm_path_index];
        let output_indices = vec![obj_path_index];

        tasks.push(Task {
            entry_name: info.name[entry_index],
            part_name: part_name_idx,
            inputs: input_indices,
            outputs: output_indices,
            command: Some(Command::CompileToObject(CompileToObject {
                cxx: config.cxx,
                cxxflags: config.cxxflags,
                input: cc_path,
                output: obj_path,
            })),
            entry_index,
        })
    }

    let lib_output_path = build_path.join(format!("lib{}.a", entry_name));
    let lib_output_path_idx = interner.insert(lib_output_path.to_str().unwrap());
    let lib_output_path = interner.get(lib_output_path_idx);

    let input_indices = obj_path_indices;
    let output_indices = vec![lib_output_path_idx];

    tasks.push(Task {
        entry_name: info.name[entry_index],
        part_name: interner.insert("@ARCHIVE_LIB@"),
        inputs: input_indices,
        outputs: output_indices,
        command: Some(Command::ArchiveLib(ArchiveLib {
            ar: config.ar,
            output: lib_output_path,
            inputs: obj_paths,
        })),
        entry_index,
    });
}

fn plan_bin_build(
    config: &Config,
    subcommand: &Subcommand,
    interner: &mut Interner,
    tasks: &mut Vec<Task>,
    info: &BuildInfo,
    entry_index: usize,
    lib_paths: &'static [&'static str],
    lib_path_indices: &[Str],
    is_test: bool,
) {
    // depend on self lib (.a)
    // compile cc to .o
    // do linking

    let artifact_kind_name = if is_test { "test" } else { "bin" };

    let entry_name = interner.get(info.name[entry_index]);

    let self_pcm_path = config
        .build_path
        .join("mod")
        .join(format!("{}.pcm", entry_name));
    let self_pcm_path_idx = interner.insert(self_pcm_path.to_str().unwrap());

    let bins = if is_test {
        &info.test[entry_index]
    } else {
        &info.bin[entry_index]
    };
    let build_path = config.build_path.join(entry_name).join(artifact_kind_name);
    let src_path = config.src_path.join(entry_name).join(artifact_kind_name);

    std::fs::create_dir_all(&build_path).unwrap();

    for &bin_name_idx in bins.iter() {
        // emit obj compilation task
        // emit linking task

        let bin_name = interner.get(bin_name_idx);

        let cc_path = src_path.join(format!("{}.cc", bin_name));
        let cc_path_index = interner.insert(cc_path.to_str().unwrap());
        let cc_path = interner.get(cc_path_index);

        let obj_path = build_path.join(format!("{}.o", bin_name));
        let obj_path_index = interner.insert(obj_path.to_str().unwrap());
        let obj_path = interner.get(obj_path_index);

        let exe_path = build_path.join(bin_name);
        let exe_path_index = interner.insert(exe_path.to_str().unwrap());
        let exe_path = interner.get(exe_path_index);

        let stamp_path = build_path.join("stamp").join(bin_name);
        let stamp_path_index = interner.insert(stamp_path.to_str().unwrap());

        let input_indices = vec![self_pcm_path_idx];
        let output_indices = vec![obj_path_index];

        tasks.push(Task {
            entry_name: info.name[entry_index],
            part_name: bin_name_idx,
            inputs: input_indices,
            outputs: output_indices,
            command: Some(Command::CompileToObject(CompileToObject {
                cxx: config.cxx,
                cxxflags: config.cxxflags,
                input: cc_path,
                output: obj_path,
            })),
            entry_index,
        });

        let mut input_indices = lib_path_indices.to_vec();
        input_indices.push(obj_path_index);
        let output_indices = vec![exe_path_index];

        tasks.push(Task {
            entry_name: info.name[entry_index],
            part_name: bin_name_idx,
            inputs: input_indices,
            outputs: output_indices,
            command: Some(Command::LinkBinary(LinkBinary {
                cxx: config.cxx,
                ldflags: config.ldflags,
                output: exe_path,
                object: obj_path,
                deps: lib_paths,
            })),
            entry_index,
        });

        if let Some(args) = should_run(subcommand, is_test, entry_name, bin_name) {
            let input_indices = vec![exe_path_index];
            let output_indices = vec![stamp_path_index];

            tasks.push(Task {
                entry_name: info.name[entry_index],
                part_name: bin_name_idx,
                inputs: input_indices,
                outputs: output_indices,
                command: Some(Command::RunBinary(RunBinary {
                    path: exe_path,
                    args,
                })),
                entry_index,
            });
        }
    }
}

fn collect_lib_path_list(
    config: &Config,
    interner: &mut Interner,
    info: &BuildInfo,
    entry_index: usize,
) -> &'static [&'static str] {
    let mut path_list: Vec<&'static str> = Vec::new();

    let self_name_idx = info.name[entry_index];

    let mut processed: HashSet<Str> = HashSet::new();
    let mut worklist: Vec<Str> = vec![self_name_idx];
    while let Some(name_idx) = worklist.pop() {
        let pos_idx: usize = usize::try_from(*info.map.get(&name_idx).unwrap()).unwrap();

        let name = interner.get(name_idx);

        let lib_path = config
            .build_path
            .join(name)
            .join("lib")
            .join(format!("lib{}.a", name));
        let lib_path_idx = interner.insert(lib_path.to_str().unwrap());
        path_list.push(interner.get(lib_path_idx));

        for &dep_name_idx in info.deps[pos_idx].iter() {
            // check deps don't cycle back to the initial name
            assert_ne!(
                dep_name_idx,
                self_name_idx,
                "Found cyclic dependency between entries '{}' and '{}'.",
                interner.get(self_name_idx),
                interner.get(name_idx),
            );
            // deduplicate using the set, so we don't reprocess same names again
            if processed.insert(dep_name_idx) {
                worklist.push(dep_name_idx);
            }
        }
    }

    path_list.leak()
}

fn plan_subfolder_fmt(
    config: &Config,
    interner: &mut Interner,
    entry_index: usize,
    entry_name: Str,
    sub_name: &str,
    parts: &[Str],
    extension: &str,
    tasks: &mut Vec<Task>,
) {
    for &part_name in parts.iter() {
        let src_path = config
            .src_path
            .join(interner.get(entry_name))
            .join(sub_name)
            .join(format!("{}.{}", interner.get(part_name), extension));
        let src_path_index = interner.insert(src_path.to_str().unwrap());
        let src_path = interner.get(src_path_index);

        tasks.push(Task {
            entry_name,
            part_name,
            inputs: Vec::new(),
            outputs: Vec::new(),
            command: Some(Command::Format(Format {
                clang_format: config.clang_format,
                path: src_path,
            })),
            entry_index,
        });
    }
}

fn plan_fmt(config: &Config, interner: &mut Interner, info: &BuildInfo) -> Vec<Task> {
    let mut tasks = Vec::new();

    for eidx in 0..info.name.len() {
        let entry_name = info.name[eidx];

        plan_subfolder_fmt(
            config,
            interner,
            eidx,
            entry_name,
            "lib",
            &info.lib[eidx],
            "cc",
            &mut tasks,
        );
        plan_subfolder_fmt(
            config,
            interner,
            eidx,
            entry_name,
            "mod",
            &info.module[eidx],
            "cppm",
            &mut tasks,
        );
        plan_subfolder_fmt(
            config,
            interner,
            eidx,
            entry_name,
            "bin",
            &info.bin[eidx],
            "cc",
            &mut tasks,
        );
        plan_subfolder_fmt(
            config,
            interner,
            eidx,
            entry_name,
            "test",
            &info.test[eidx],
            "cc",
            &mut tasks,
        );
    }

    tasks
}

// CACHING TODO:
// -> Define cache state.
// -> Implement cache state loading and storing.
// -> Implement cache pruning based on size and entry count limit. Use "last used" timestamps to
//    decide what to remove.
// -> Planning should actually update the "last used" timestamps in cache state if it emits CacheLoad tasks for
//    the item.
// -> modify the existing planning functions to account for cache_state + DeepHashes.

// "Parallel" struct to Build Info.
// Hash the corresponding deephash for output artifacts including modules, librari archives, object files etc.
//
// Is intended to be used in planning along with the cache state to decide to emit cache load or real compilation + cache store.
//
// A "deep" hash is defined roughly as aggregate of:
// - hash of input source file path if there is a specific input source file.
// - hash of input source file content, if there is an input source file.
// - "deep" hashes of the module files that this file depends on in the same entry.
// - "deep" hashes of other entries that the current entry depends on.
// - "deep" hash of the build configuration including anything that effects the behavior of the build.
// - hash of self binary byte content, which is the binary of `zxbs`.
//    Basically just read "/proc/self/exe" and hash it.
pub struct DeepHashes {
    pub entry_pcm: Vec<u64>,
    pub entry_lib_archive: Vec<u64>,
    pub module_pcm: Vec<Vec<u64>>,
    pub module_object: Vec<Vec<u64>>,
    pub lib_object: Vec<Vec<u64>>,
    pub bin_object: Vec<Vec<u64>>,
    pub bin_exe: Vec<Vec<u64>>,
    pub test_object: Vec<Vec<u64>>,
    pub test_exe: Vec<Vec<u64>>,
    pub test_stamp: Vec<Vec<u64>>,
}

impl DeepHashes {
    fn new(len: usize) -> DeepHashes {
        DeepHashes {
            entry_pcm: vec![0; len],
            entry_lib_archive: vec![0; len],
            module_pcm: vec![Vec::new(); len],
            module_object: vec![Vec::new(); len],
            lib_object: vec![Vec::new(); len],
            bin_object: vec![Vec::new(); len],
            bin_exe: vec![Vec::new(); len],
            test_object: vec![Vec::new(); len],
            test_exe: vec![Vec::new(); len],
            test_stamp: vec![Vec::new(); len],
        }
    }
}

fn construct_base_deep_hash(config: &Config, self_exe_hash: u64) -> u64 {
    let mut b = HashBuilder::new();

    // Try to include EVERYTHING that might effect the generated artifacts.
    b.update(config.cxx.as_bytes());
    b.update(config.ar.as_bytes());
    b.update(config.clang_scan_deps.as_bytes());

    for flags in [config.cxxflags, config.ldflags] {
        for flag in flags {
            b.update(flag.as_bytes());
        }
    }

    b.update(&self_exe_hash.to_le_bytes());

    b.digest()
}

fn construct_module_deep_hashes(
    config: &Config,
    interner: &mut Interner,
    info: &BuildInfo,
    entry_idx: usize,
    out: &mut DeepHashes,
    hash_builder: &mut HashBuilder,
) {
    let entry_name = interner.get(info.name[entry_idx]);

    let parts = &info.module[entry_idx];
    let num_parts = parts.len();

    assert!(num_parts > 0);

    let mut remaining_deps: Vec<Vec<usize>> = vec![Vec::new(); num_parts];
    let mut rev_deps: Vec<Vec<usize>> = vec![Vec::new(); num_parts];

    let mod_dep_matrix = &info.mod_dep_matrix[entry_idx];
    assert_eq!(mod_dep_matrix.len(), num_parts * num_parts);

    let mut backlog: Vec<usize> = Vec::new();

    for part_idx in 0..num_parts {
        let start = part_idx * num_parts;
        let deps = &mod_dep_matrix[start..start + num_parts];

        for (dep_idx, &is_dep) in deps.iter().enumerate() {
            if is_dep {
                assert_ne!(dep_idx, part_idx);

                remaining_deps[part_idx].push(dep_idx);
                rev_deps[dep_idx].push(part_idx);
            }
        }

        if remaining_deps[part_idx].is_empty() {
            backlog.push(part_idx);
        }
    }

    let rev_deps = rev_deps;

    let mut num_processed = 0;

    out.module_pcm[entry_idx] = vec![0; num_parts];
    out.module_object[entry_idx] = vec![0; num_parts];

    let start_hash = hash_builder.digest();

    let mut part_was_visited = vec![false; num_parts];

    assert!(!backlog.is_empty());
    while let Some(part_idx) = backlog.pop() {
        assert!(!part_was_visited[part_idx]);

        let path = config.src_path.join(entry_name).join("mod").join(format!(
            "{}.cppm",
            interner.get(info.module[entry_idx][part_idx])
        ));
        let path = interner.insert(path.to_str().unwrap());
        let basic_hash = *info.basic_hash.get(&path).unwrap();

        let mut hb = HashBuilder::new();
        hb.update(&start_hash.to_le_bytes());
        hb.update(&basic_hash.to_le_bytes());

        let mdm_start = part_idx * num_parts;
        for (dep_idx, &is_dep) in mod_dep_matrix[mdm_start..mdm_start + num_parts]
            .iter()
            .enumerate()
        {
            if is_dep {
                assert_ne!(dep_idx, part_idx);
                assert!(part_was_visited[dep_idx]);
                hb.update(&out.module_pcm[entry_idx][dep_idx].to_le_bytes());
            }
        }

        let pcm_hash = hb.digest();

        // We can't have object and pcm hashes be the same.
        hb.update(b"object");

        let obj_hash = hb.digest();

        out.module_pcm[entry_idx][part_idx] = pcm_hash;

        out.module_object[entry_idx][part_idx] = obj_hash;

        // check what else can we calculate now that we calculate hash for this part
        for &dependee_idx in rev_deps[part_idx].iter() {
            let rem_deps: &mut Vec<usize> = &mut remaining_deps[dependee_idx];
            let pos = rem_deps
                .iter()
                .enumerate()
                .find(|(_, x)| **x == part_idx)
                .map(|(i, _)| i)
                .unwrap();
            rem_deps.swap_remove(pos);

            if rem_deps.is_empty() {
                backlog.push(dependee_idx);
            }
        }

        num_processed += 1;
        part_was_visited[part_idx] = true;
    }

    for part_idx in 0..num_parts {
        hash_builder.update(&out.module_object[entry_idx][part_idx].to_le_bytes());
    }

    assert_eq!(num_processed, num_parts);
}

fn construct_entry_deep_hash(
    config: &Config,
    interner: &mut Interner,
    info: &BuildInfo,
    base_hash: u64,
    entry_idx: usize,
    entry_was_visited: &[bool],
    out: &mut DeepHashes,
) {
    let entry_name = interner.get(info.name[entry_idx]);

    // It is important to be careful with how we chain the hashes.
    // Making the chaining too conservative might cause unneeded recompilations even when we
    // have the right data in the cache. Or it might cause miscompilations if it is too
    // ambitious.

    let mut hash_builder = HashBuilder::new();
    hash_builder.update(&base_hash.to_le_bytes());

    // start with the interface (pcm) hash aggregation we will add lib (object) stuff later.

    for dep_name in info.deps[entry_idx].iter() {
        let dep_idx = usize::try_from(*info.map.get(dep_name).unwrap()).unwrap();
        let dep_hash = out.entry_pcm[dep_idx];
        assert!(entry_was_visited[dep_idx]);
        hash_builder.update(&dep_hash.to_le_bytes());
    }

    construct_module_deep_hashes(config, interner, info, entry_idx, out, &mut hash_builder);

    // Change hash deterministically so we don't have a case where this hash matches with some
    // other hash. This would happen if the function that constructed the module hashes above just
    // read hash_builder.digest() and used it for something but didn't do any updates before returning.
    //
    // It is technically OK for this hash to be the same hash with some other hash as it is just a meta hash.
    // But we want to avoid it anyway so we don't get surprised in some other context.
    hash_builder.update(b"entry pcm");

    // save entry pcm hash
    let entry_pcm_hash = hash_builder.digest();
    out.entry_pcm[entry_idx] = entry_pcm_hash;

    // Hash tracking gets a bit more complex after this point.
    //
    // We only processed the cppm files up to here and they all depend on each other in a
    // simple way.
    //
    // Object files only depend on the cppm (pcm) files but don't depend on other object files.
    // Meaning a change on the cc file or the cppm (pcm) files is the only cause that might change
    // the object file output.
    //
    // So object file hashes should be independent of each other but all dependent on the entry_pcm_hash
    // we calculated.
    //
    // Library archive depends on all the object files in this module.
    //
    // Executable object files also only depend on the cppm (pcm) files so they will be based on entry_pcm_hash +
    // basic hash of the corresponding cc file.
    //
    // Executable files themselves are linked using all libraries that are depended and the object file that the cc
    // file output so they will depend on the interface (pcm), library (.a, object .o) files.
    //
    // Nothing depends on the object files that are for executables or the executables themselves so we can just forget
    // their hashes wihout incorporating them into our hash accumulation.
    //
    // hash_builder will only be used for the library hash after this point. Meaning we will use it for only
    // library archive file hash and the linked executables' hashes.
    //
    // The object files' hashes won't be based on hash_builder at all, they will just combine their basic_hash with
    // entry_pcm_hash to calculate their deep hash.

    // remember that the lib (.a) output of this entry also doesn't depend on the lib outputs of
    // the dependencies so we will delay incorporating lib hashes of entry dependencies until after we calculated
    // hash for lib output of this entry.

    // lib
    let mut lib_hashes = Vec::with_capacity(info.lib[entry_idx].len());
    for &lib_part_name in info.lib[entry_idx].iter() {
        let path = config
            .src_path
            .join(entry_name)
            .join("lib")
            .join(format!("{}.cc", interner.get(lib_part_name)));
        let path = interner.insert(path.to_str().unwrap());
        let basic_hash = *info.basic_hash.get(&path).unwrap();

        // mix entry_pcm_hash with basic_hash
        let mut hb = HashBuilder::new();
        hb.update(&entry_pcm_hash.to_le_bytes());
        hb.update(&basic_hash.to_le_bytes());

        let obj_hash = hb.digest();
        lib_hashes.push(obj_hash);

        hash_builder.update(&obj_hash.to_le_bytes());
    }
    out.lib_object[entry_idx] = lib_hashes;

    // save entry lib archive hash
    let entry_lib_hash = hash_builder.digest();
    out.entry_lib_archive[entry_idx] = entry_lib_hash;

    // incorporate the lib hashes of entry dependencies into hash_builder
    // hash builder will only be used for linked executable output hashes after this point.
    for dep_name in info.deps[entry_idx].iter() {
        let dep_idx = usize::try_from(*info.map.get(dep_name).unwrap()).unwrap();
        assert!(entry_was_visited[dep_idx]);
        let dep_hash = out.entry_lib_archive[dep_idx];
        hash_builder.update(&dep_hash.to_le_bytes());
    }

    let link_base_hash = hash_builder.digest();

    // test
    let test_info = &info.test[entry_idx];
    let mut test_object_hashes = Vec::with_capacity(test_info.len());
    let mut test_exe_hashes = Vec::with_capacity(test_info.len());
    let mut test_stamp_hashes = Vec::with_capacity(test_info.len());

    for &test_name in info.test[entry_idx].iter() {
        let path = config
            .src_path
            .join(entry_name)
            .join("test")
            .join(format!("{}.cc", interner.get(test_name)));
        let path = interner.insert(path.to_str().unwrap());
        let basic_hash = *info.basic_hash.get(&path).unwrap();

        let mut hb = HashBuilder::new();
        hb.update(&basic_hash.to_le_bytes());
        hb.update(&entry_pcm_hash.to_le_bytes());

        test_object_hashes.push(hb.digest());

        hb.update(&link_base_hash.to_le_bytes());

        test_exe_hashes.push(hb.digest());

        hb.update(b"stamp");

        test_stamp_hashes.push(hb.digest());
    }

    out.test_object[entry_idx] = test_object_hashes;
    out.test_exe[entry_idx] = test_exe_hashes;
    out.test_stamp[entry_idx] = test_stamp_hashes;

    // bin

    let bin_info = &info.bin[entry_idx];
    let mut bin_object_hashes = Vec::with_capacity(bin_info.len());
    let mut bin_exe_hashes = Vec::with_capacity(bin_info.len());

    for &bin_name in info.bin[entry_idx].iter() {
        let path = config
            .src_path
            .join(entry_name)
            .join("bin")
            .join(format!("{}.cc", interner.get(bin_name)));
        let path = interner.insert(path.to_str().unwrap());
        let basic_hash = *info.basic_hash.get(&path).unwrap();

        let mut hb = HashBuilder::new();
        hb.update(&basic_hash.to_le_bytes());
        hb.update(&entry_pcm_hash.to_le_bytes());

        bin_object_hashes.push(hb.digest());

        hb.update(&link_base_hash.to_le_bytes());

        bin_exe_hashes.push(hb.digest());
    }

    out.bin_object[entry_idx] = bin_object_hashes;
    out.bin_exe[entry_idx] = bin_exe_hashes;
}

// Construct "deep" hashes from "basic" hashes by following the dependency graph.
pub fn construct_deep_hashes(
    config: &Config,
    interner: &mut Interner,
    info: &BuildInfo,
) -> DeepHashes {
    let base_hash = construct_base_deep_hash(
        config,
        *info
            .basic_hash
            .get(&interner.insert("/proc/self/exe"))
            .unwrap(),
    );

    let num_entries = info.name.len();

    if num_entries == 0 {
        return DeepHashes::new(0);
    }

    let mut backlog: Vec<usize> = Vec::new();

    let mut remaining_deps: Vec<Vec<usize>> = vec![Vec::new(); num_entries];
    // this will be used to do a reverse lookup as we calcuate a deep has of an entry so
    // we want to update the `remaining deps` accordingly.
    let mut rev_deps: Vec<Vec<usize>> = vec![Vec::new(); num_entries];
    for (entry_idx, deps) in info.deps.iter().enumerate() {
        if deps.is_empty() {
            backlog.push(entry_idx);
        }

        for dep_name in deps.iter() {
            let dep_idx = usize::try_from(*info.map.get(dep_name).unwrap()).unwrap();
            remaining_deps[entry_idx].push(dep_idx);
            rev_deps[dep_idx].push(entry_idx);
        }
    }

    let rev_deps = rev_deps;

    let mut num_calculated = 0;

    let mut out = DeepHashes::new(num_entries);

    let mut entry_was_visited = vec![false; num_entries];

    assert!(!backlog.is_empty());
    while let Some(entry_idx) = backlog.pop() {
        assert!(remaining_deps[entry_idx].is_empty());
        assert!(!entry_was_visited[entry_idx]);
        construct_entry_deep_hash(
            config,
            interner,
            info,
            base_hash,
            entry_idx,
            &entry_was_visited,
            &mut out,
        );
        entry_was_visited[entry_idx] = true;

        for &dependee_idx in rev_deps[entry_idx].iter() {
            let rem_deps: &mut Vec<usize> = &mut remaining_deps[dependee_idx];
            let pos = rem_deps
                .iter()
                .enumerate()
                .find(|(_, x)| **x == entry_idx)
                .map(|(i, _)| i)
                .unwrap();
            rem_deps.swap_remove(pos);

            if rem_deps.is_empty() {
                backlog.push(dependee_idx);
            }
        }

        num_calculated += 1;
    }

    assert_eq!(num_calculated, num_entries);

    out
}

pub fn plan(
    config: &Config,
    subcommand: &Subcommand,
    interner: &mut Interner,
    info: &BuildInfo,
    cache_state: &mut CacheState,
) -> Vec<Task> {
    assert_eq!(info.name.len(), info.mod_dep_matrix.len());
    assert_eq!(info.name.len(), info.module.len());
    assert_eq!(info.name.len(), info.lib.len());
    assert_eq!(info.name.len(), info.bin.len());
    assert_eq!(info.name.len(), info.test.len());
    assert_eq!(info.name.len(), info.deps.len());

    if subcommand == &Subcommand::Fmt {
        return plan_fmt(config, interner, info);
    }

    let deep_hashes = construct_deep_hashes(config, interner, info);

    let mut tasks = Vec::new();

    for eidx in 0..info.name.len() {
        let params = Params {
            config,
            subcommand,
            interner,
            info,
            cache_state,
            entry_idx: eidx,
            deep_hashes: &deep_hashes,
            tasks: &mut tasks,
        };

        plan_module_build(params);
        plan_lib_build(params);

        // collect the library path list for doing static linking.
        let lib_path_list = collect_lib_path_list(config, interner, info, eidx);

        let lib_path_indices: Vec<Str> = lib_path_list.iter().map(|p| interner.insert(p)).collect();

        plan_test_build(params, &lib_path_indices);
        plan_bin_build(params, &lib_path_indices);

        plan_module_build(config, interner, &mut tasks, info, eidx);
        plan_lib_build(config, interner, &mut tasks, info, eidx);
    }

    tasks
}
