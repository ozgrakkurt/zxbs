// Implementation of the "plan" stage.
//
// This stage takes the input from the "scan" stage and creates the output to be used by the "build" stage.
//
// Don't do IO in this code!!

use crate::build::Task;
use crate::cache::CacheState;
use crate::command::{
    ArchiveLib, CacheLoad, CacheStore, Command, CompileModule, CompileToObject, Format, LinkBinary,
    RunBinary, RunTest,
};
use crate::config::{Config, Subcommand};
use crate::hash::{Hash, HashSet, UnorderedHashCombiner, hash_bytes, hash_ordered, mix_hashes};
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
    entry_index: usize,
    deep_hashes: &'a DeepHashes,
    tasks: &'a mut Vec<Task>,
    timestamp: u64,
}

impl<'a> Params<'a> {
    // Some RUST SPECIAL here. We have to do this because we can't copy/clone this struct.
    // Reborrow all `&mut` fields with a shorter lifetime.
    // This is the "clone" you're looking for.
    pub fn reborrow<'b>(&'b mut self) -> Params<'b> {
        Params {
            config: self.config,
            subcommand: self.subcommand,
            interner: &mut *self.interner, // reborrow
            info: self.info,
            cache_state: &mut *self.cache_state, // reborrow
            entry_index: self.entry_index,
            deep_hashes: self.deep_hashes,
            tasks: &mut *self.tasks, // reborrow
            timestamp: self.timestamp,
        }
    }
}

fn plan_module_build(
    Params {
        config,
        subcommand: _,
        interner,
        info,
        cache_state,
        entry_index,
        deep_hashes,
        tasks,
        timestamp,
    }: Params,
) {
    let entry_name_idx = info.name[entry_index];
    let entry_name = interner.get(entry_name_idx);

    let mod_parts = info.module[entry_index].as_slice();
    let n_mod_parts = mod_parts.len();

    let entry_deps = info.deps[entry_index].as_slice();

    let mut ext_pcm_paths: Vec<Str> = Vec::with_capacity(entry_deps.len());
    let mod_build_path = config.build_path.join("mod");
    for &dep_name in entry_deps.iter() {
        let dep_name = interner.get(dep_name);
        let dep_pcm_path = mod_build_path.join(format!("{}.pcm", dep_name));
        let dep_pcm_path = interner.insert(dep_pcm_path.to_str().unwrap());
        ext_pcm_paths.push(dep_pcm_path);
    }

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

    let pcm_hashes = &deep_hashes.module_pcm[entry_index];
    let obj_hashes = &deep_hashes.module_object[entry_index];

    let cache_files_path = config.cache_path.to_str().unwrap();

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

        let pcm_path = local_pcm_paths[part_idx];
        let obj_path = local_object_paths[part_idx];
        let output_indices = vec![pcm_path, obj_path];

        let cppm_path = mod_src_path.join(format!("{}.cppm", interner.get(part)));
        let cppm_path = interner.insert(cppm_path.to_str().unwrap());
        let cppm_path = interner.get(cppm_path);

        let pcm_hash = pcm_hashes[part_idx];
        let obj_hash = obj_hashes[part_idx];

        if let (Some(pcm_output_hash), Some(obj_output_hash)) = (
            cache_state.item(pcm_hash, timestamp),
            cache_state.item(obj_hash, timestamp),
        ) {
            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: part,
                inputs: Vec::new(),
                outputs: vec![pcm_path],
                command: Some(Command::CacheLoad(CacheLoad {
                    cache_files_path,
                    output_path: interner.get(pcm_path),
                    deep_hash: pcm_hash,
                    expected_hash: pcm_output_hash,
                })),
                entry_index,
            });
            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: part,
                inputs: Vec::new(),
                outputs: vec![obj_path],
                command: Some(Command::CacheLoad(CacheLoad {
                    cache_files_path,
                    output_path: interner.get(obj_path),
                    deep_hash: obj_hash,
                    expected_hash: obj_output_hash,
                })),
                entry_index,
            });
        } else {
            tasks.push(Task {
                entry_name: entry_name_idx,
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

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: part,
                inputs: vec![pcm_path],
                outputs: Vec::new(),
                command: Some(Command::CacheStore(CacheStore {
                    cache_files_path,
                    file_path: interner.get(pcm_path),
                    deep_hash: pcm_hash,
                })),
                entry_index,
            });

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: part,
                inputs: vec![obj_path],
                outputs: Vec::new(),
                command: Some(Command::CacheStore(CacheStore {
                    cache_files_path,
                    file_path: interner.get(obj_path),
                    deep_hash: obj_hash,
                })),
                entry_index,
            });
        }
    }
}

fn plan_lib_build(
    Params {
        config,
        subcommand: _,
        interner,
        info,
        cache_state,
        entry_index,
        deep_hashes,
        tasks,
        timestamp,
    }: Params,
) {
    // build object files (both .cc and .s)
    // archive into .a file

    let entry_name_idx = info.name[entry_index];
    let entry_name = interner.get(entry_name_idx);

    let parts = &info.lib[entry_index];
    let src_path = config.src_path.join(entry_name).join("lib");
    let build_path = config.build_path.join(entry_name).join("lib");

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

    let obj_hashes = &deep_hashes.lib_object[entry_index];

    let cache_files_path = config.cache_path.to_str().unwrap();

    for (part_idx, &part_name_idx) in parts.iter().enumerate() {
        let part_name = interner.get(part_name_idx);
        let cc_path = src_path.join(format!("{}.cc", part_name));
        let cc_path_index = interner.insert(cc_path.to_str().unwrap());
        let cc_path = interner.get(cc_path_index);
        let obj_path = build_path.join(format!("{}.o", part_name));
        let obj_path_index = interner.insert(obj_path.to_str().unwrap());
        let obj_path = interner.get(obj_path_index);

        obj_path_indices.push(obj_path_index);
        obj_paths.push(interner.get(obj_path_index));

        let obj_hash = obj_hashes[part_idx];

        if let Some(obj_output_hash) = cache_state.item(obj_hash, timestamp) {
            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: part_name_idx,
                inputs: Vec::new(),
                outputs: vec![obj_path_index],
                command: Some(Command::CacheLoad(CacheLoad {
                    cache_files_path,
                    output_path: obj_path,
                    deep_hash: obj_hash,
                    expected_hash: obj_output_hash,
                })),
                entry_index,
            });
        } else {
            let input_indices = vec![root_pcm_path_index];
            let output_indices = vec![obj_path_index];

            tasks.push(Task {
                entry_name: entry_name_idx,
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
            });

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: part_name_idx,
                inputs: vec![obj_path_index],
                outputs: Vec::new(),
                command: Some(Command::CacheStore(CacheStore {
                    cache_files_path,
                    file_path: obj_path,
                    deep_hash: obj_hash,
                })),
                entry_index,
            });
        }
    }

    let lib_output_path = build_path.join(format!("lib{}.a", entry_name));
    let lib_output_path_idx = interner.insert(lib_output_path.to_str().unwrap());
    let lib_output_path = interner.get(lib_output_path_idx);

    let lib_archive_hash = deep_hashes.entry_lib_archive[entry_index];

    // this is only used for tracing so nothing special.
    let lib_archive_part_name = interner.insert("@ARCHIVE_LIB@");

    if let Some(lib_archive_output_hash) = cache_state.item(lib_archive_hash, timestamp) {
        tasks.push(Task {
            entry_name: entry_name_idx,
            part_name: lib_archive_part_name,
            inputs: Vec::new(),
            outputs: vec![lib_output_path_idx],
            command: Some(Command::CacheLoad(CacheLoad {
                cache_files_path,
                output_path: lib_output_path,
                deep_hash: lib_archive_hash,
                expected_hash: lib_archive_output_hash,
            })),
            entry_index,
        });
    } else {
        let input_indices = obj_path_indices;
        let output_indices = vec![lib_output_path_idx];

        tasks.push(Task {
            entry_name: entry_name_idx,
            part_name: lib_archive_part_name,
            inputs: input_indices,
            outputs: output_indices,
            command: Some(Command::ArchiveLib(ArchiveLib {
                ar: config.ar,
                output: lib_output_path,
                inputs: obj_paths,
            })),
            entry_index,
        });

        tasks.push(Task {
            entry_name: entry_name_idx,
            part_name: lib_archive_part_name,
            inputs: vec![lib_output_path_idx],
            outputs: Vec::new(),
            command: Some(Command::CacheStore(CacheStore {
                cache_files_path,
                file_path: lib_output_path,
                deep_hash: lib_archive_hash,
            })),
            entry_index,
        });
    }
}

fn plan_bin_build(
    Params {
        config,
        subcommand,
        interner,
        info,
        cache_state,
        entry_index,
        deep_hashes,
        tasks,
        timestamp,
    }: Params,
    lib_paths: &'static [&'static str],
    lib_path_indices: &[Str],
) {
    // depend on self lib (.a)
    // compile cc to .o
    // do linking

    let entry_name_idx = info.name[entry_index];
    let entry_name = interner.get(entry_name_idx);

    let self_pcm_path = config
        .build_path
        .join("mod")
        .join(format!("{}.pcm", entry_name));
    let self_pcm_path_idx = interner.insert(self_pcm_path.to_str().unwrap());

    let bins = &info.bin[entry_index];
    let build_path = config.build_path.join(entry_name).join("bin");
    let src_path = config.src_path.join(entry_name).join("bin");

    let obj_hashes = &deep_hashes.bin_object[entry_index];
    let exe_hashes = &deep_hashes.bin_exe[entry_index];

    let cache_files_path = config.cache_path.to_str().unwrap();

    for (bin_idx, &bin_name_idx) in bins.iter().enumerate() {
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

        let obj_hash = obj_hashes[bin_idx];
        let exe_hash = exe_hashes[bin_idx];

        if let Some(obj_output_hash) = cache_state.item(obj_hash, timestamp) {
            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: bin_name_idx,
                inputs: Vec::new(),
                outputs: vec![obj_path_index],
                command: Some(Command::CacheLoad(CacheLoad {
                    cache_files_path,
                    output_path: obj_path,
                    deep_hash: obj_hash,
                    expected_hash: obj_output_hash,
                })),
                entry_index,
            });
        } else {
            let input_indices = vec![self_pcm_path_idx];
            let output_indices = vec![obj_path_index];

            tasks.push(Task {
                entry_name: entry_name_idx,
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

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: bin_name_idx,
                inputs: vec![obj_path_index],
                outputs: Vec::new(),
                command: Some(Command::CacheStore(CacheStore {
                    cache_files_path,
                    file_path: obj_path,
                    deep_hash: obj_hash,
                })),
                entry_index,
            });
        }

        if let Some(exe_output_hash) = cache_state.item(exe_hash, timestamp) {
            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: bin_name_idx,
                inputs: Vec::new(),
                outputs: vec![exe_path_index],
                command: Some(Command::CacheLoad(CacheLoad {
                    cache_files_path,
                    output_path: exe_path,
                    deep_hash: exe_hash,
                    expected_hash: exe_output_hash,
                })),
                entry_index,
            });
        } else {
            let mut input_indices = lib_path_indices.to_vec();
            input_indices.push(obj_path_index);
            let output_indices = vec![exe_path_index];

            tasks.push(Task {
                entry_name: entry_name_idx,
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

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: bin_name_idx,
                inputs: vec![exe_path_index],
                outputs: Vec::new(),
                command: Some(Command::CacheStore(CacheStore {
                    cache_files_path,
                    file_path: exe_path,
                    deep_hash: exe_hash,
                })),
                entry_index,
            });
        }

        if let Some(args) = should_run_binary(subcommand, entry_name, bin_name) {
            let input_indices = vec![exe_path_index];
            let output_indices = vec![stamp_path_index];

            tasks.push(Task {
                entry_name: entry_name_idx,
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

fn plan_test_build(
    Params {
        config,
        subcommand,
        interner,
        info,
        cache_state,
        entry_index,
        deep_hashes,
        tasks,
        timestamp,
    }: Params,
    lib_paths: &'static [&'static str],
    lib_path_indices: &[Str],
) {
    // depend on self lib (.a)
    // compile cc to .o
    // do linking

    let entry_name_idx = info.name[entry_index];
    let entry_name = interner.get(entry_name_idx);

    let self_pcm_path = config
        .build_path
        .join("mod")
        .join(format!("{}.pcm", entry_name));
    let self_pcm_path_idx = interner.insert(self_pcm_path.to_str().unwrap());

    let tests = &info.test[entry_index];

    let build_path = config.build_path.join(entry_name).join("test");
    let src_path = config.src_path.join(entry_name).join("test");

    let obj_hashes = &deep_hashes.test_object[entry_index];
    let exe_hashes = &deep_hashes.test_exe[entry_index];
    let stamp_hashes = &deep_hashes.test_stamp[entry_index];

    let cache_files_path = config.cache_path.to_str().unwrap();

    for (test_idx, &test_name_idx) in tests.iter().enumerate() {
        // emit obj compilation task
        // emit linking task

        let test_name = interner.get(test_name_idx);

        let cc_path = src_path.join(format!("{}.cc", test_name));
        let cc_path_index = interner.insert(cc_path.to_str().unwrap());
        let cc_path = interner.get(cc_path_index);

        let obj_path = build_path.join(format!("{}.o", test_name));
        let obj_path_index = interner.insert(obj_path.to_str().unwrap());
        let obj_path = interner.get(obj_path_index);

        let exe_path = build_path.join(test_name);
        let exe_path_index = interner.insert(exe_path.to_str().unwrap());
        let exe_path = interner.get(exe_path_index);

        let stamp_path = build_path.join("stamp").join(test_name);
        let stamp_path_index = interner.insert(stamp_path.to_str().unwrap());
        let stamp_path = interner.get(stamp_path_index);

        let obj_hash = obj_hashes[test_idx];
        let exe_hash = exe_hashes[test_idx];
        let stamp_hash = stamp_hashes[test_idx];

        if let Some(obj_output_hash) = cache_state.item(obj_hash, timestamp) {
            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: test_name_idx,
                inputs: Vec::new(),
                outputs: vec![obj_path_index],
                command: Some(Command::CacheLoad(CacheLoad {
                    cache_files_path,
                    output_path: obj_path,
                    deep_hash: obj_hash,
                    expected_hash: obj_output_hash,
                })),
                entry_index,
            });
        } else {
            let input_indices = vec![self_pcm_path_idx];
            let output_indices = vec![obj_path_index];

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: test_name_idx,
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

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: test_name_idx,
                inputs: vec![obj_path_index],
                outputs: Vec::new(),
                command: Some(Command::CacheStore(CacheStore {
                    cache_files_path,
                    file_path: obj_path,
                    deep_hash: obj_hash,
                })),
                entry_index,
            });
        }

        if let Some(exe_output_hash) = cache_state.item(exe_hash, timestamp) {
            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: test_name_idx,
                inputs: Vec::new(),
                outputs: vec![exe_path_index],
                command: Some(Command::CacheLoad(CacheLoad {
                    cache_files_path,
                    output_path: exe_path,
                    deep_hash: exe_hash,
                    expected_hash: exe_output_hash,
                })),
                entry_index,
            });
        } else {
            let mut input_indices = lib_path_indices.to_vec();
            input_indices.push(obj_path_index);
            let output_indices = vec![exe_path_index];

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: test_name_idx,
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

            tasks.push(Task {
                entry_name: entry_name_idx,
                part_name: test_name_idx,
                inputs: vec![exe_path_index],
                outputs: Vec::new(),
                command: Some(Command::CacheStore(CacheStore {
                    cache_files_path,
                    file_path: exe_path,
                    deep_hash: exe_hash,
                })),
                entry_index,
            });
        }

        if should_run_test(subcommand, entry_name, test_name) {
            if let Some(stamp_output_hash) = cache_state.item(stamp_hash, timestamp) {
                tasks.push(Task {
                    entry_name: entry_name_idx,
                    part_name: test_name_idx,
                    inputs: Vec::new(),
                    outputs: vec![stamp_path_index],
                    command: Some(Command::CacheLoad(CacheLoad {
                        cache_files_path,
                        output_path: stamp_path,
                        deep_hash: stamp_hash,
                        expected_hash: stamp_output_hash,
                    })),
                    entry_index,
                });
            } else {
                let input_indices = vec![exe_path_index];
                let output_indices = vec![stamp_path_index];

                tasks.push(Task {
                    entry_name: entry_name_idx,
                    part_name: test_name_idx,
                    inputs: input_indices,
                    outputs: output_indices,
                    command: Some(Command::RunTest(RunTest {
                        path: exe_path,
                        stamp_hash,
                        stamp_path,
                    })),
                    entry_index,
                });

                tasks.push(Task {
                    entry_name: entry_name_idx,
                    part_name: test_name_idx,
                    inputs: vec![stamp_path_index],
                    outputs: Vec::new(),
                    command: Some(Command::CacheStore(CacheStore {
                        cache_files_path,
                        file_path: stamp_path,
                        deep_hash: stamp_hash,
                    })),
                    entry_index,
                });
            }
        }
    }
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

// "Parallel" struct to Build Info.
// Hash the corresponding deephash for output artifacts including modules, library archives, object files etc.
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
    pub entry_pcm: Vec<Hash>,
    pub entry_lib_archive: Vec<Hash>,
    pub module_pcm: Vec<Vec<Hash>>,
    pub module_object: Vec<Vec<Hash>>,
    pub lib_object: Vec<Vec<Hash>>,
    pub bin_object: Vec<Vec<Hash>>,
    pub bin_exe: Vec<Vec<Hash>>,
    pub test_object: Vec<Vec<Hash>>,
    pub test_exe: Vec<Vec<Hash>>,
    pub test_stamp: Vec<Vec<Hash>>,
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

fn construct_base_deep_hash(config: &Config, self_exe_hash: Hash) -> Hash {
    let mut bufs: Vec<&[u8]> = Vec::new();

    // Try to include everything that might effect the generated artifacts.
    bufs.push(config.cxx.as_bytes());
    bufs.push(config.ar.as_bytes());
    bufs.push(config.clang_scan_deps.as_bytes());

    for flags in config.cxxflags.iter() {
        bufs.push(flags.as_bytes());
    }
    for flags in config.ldflags.iter() {
        bufs.push(flags.as_bytes());
    }

    let self_exe_hash_bytes = self_exe_hash.to_le_bytes();
    bufs.push(&self_exe_hash_bytes);

    hash_ordered(&bufs)
}

fn construct_module_deep_hashes(
    config: &Config,
    interner: &mut Interner,
    info: &BuildInfo,
    entry_idx: usize,
    out: &mut DeepHashes,
    hash_combiner: &mut UnorderedHashCombiner,
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

    let start_hash = hash_combiner.combine();

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

        // to be used to calculate deep hash for this module part
        let mut hc = UnorderedHashCombiner::new();
        hc.add(start_hash);
        hc.add(basic_hash);

        let mdm_start = part_idx * num_parts;
        for (dep_idx, &is_dep) in mod_dep_matrix[mdm_start..mdm_start + num_parts]
            .iter()
            .enumerate()
        {
            if is_dep {
                assert_ne!(dep_idx, part_idx);
                assert!(part_was_visited[dep_idx]);
                hc.add(out.module_pcm[entry_idx][dep_idx]);
            }
        }

        let pcm_hash = hc.combine();

        // We can't have object and pcm hashes be the same so mix a deterministic value in.
        let obj_hash = mix_hashes(pcm_hash, hash_bytes(b"object"));

        out.module_pcm[entry_idx][part_idx] = pcm_hash;
        out.module_object[entry_idx][part_idx] = obj_hash;

        // check what else can we calculate now that we calculated hash for this part
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

    for &hash in out.module_object[entry_idx].iter() {
        hash_combiner.add(hash);
    }

    assert_eq!(num_processed, num_parts);
}

fn construct_entry_deep_hash(
    config: &Config,
    interner: &mut Interner,
    info: &BuildInfo,
    base_hash: Hash,
    entry_idx: usize,
    entry_was_visited: &[bool],
    out: &mut DeepHashes,
) {
    let entry_name = interner.get(info.name[entry_idx]);

    // It is important to be careful with how we chain the hashes.
    // Making the chaining too conservative might cause unneeded recompilations even when we
    // have the right data in the cache. Or it might cause miscompilations if it is too
    // ambitious.

    let mut hash_combiner = UnorderedHashCombiner::new();
    hash_combiner.add(base_hash);

    // start with the interface (pcm) hash aggregation. We will add lib (object) stuff later.

    for dep_name in info.deps[entry_idx].iter() {
        let dep_idx = usize::try_from(*info.map.get(dep_name).unwrap()).unwrap();
        assert!(entry_was_visited[dep_idx]);
        let dep_hash = out.entry_pcm[dep_idx];
        hash_combiner.add(dep_hash);
    }

    construct_module_deep_hashes(config, interner, info, entry_idx, out, &mut hash_combiner);

    // Change hash deterministically so we don't have a case where this hash matches with some
    // other hash. This would happen if the function that constructed the module hashes above just
    // read hash_builder.digest() and used it for something but didn't do any updates before returning.
    //
    // It is technically OK for this hash to be the same hash with some other hash as it is just a meta hash.
    // But we want to avoid it anyway so we don't get surprised in some other context.
    hash_combiner.add(hash_bytes(b"entry pcm"));

    // save entry pcm hash
    let entry_pcm_hash = hash_combiner.combine();
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

        let obj_hash = mix_hashes(basic_hash, entry_pcm_hash);
        lib_hashes.push(obj_hash);

        hash_combiner.add(obj_hash);
    }
    out.lib_object[entry_idx] = lib_hashes;

    // save entry lib archive hash
    let entry_lib_hash = hash_combiner.combine();
    out.entry_lib_archive[entry_idx] = entry_lib_hash;

    // incorporate the lib hashes of entry dependencies into hash_builder
    // hash builder will only be used for linked executable output hashes after this point.
    for dep_name in info.deps[entry_idx].iter() {
        let dep_idx = usize::try_from(*info.map.get(dep_name).unwrap()).unwrap();
        assert!(entry_was_visited[dep_idx]);
        let dep_hash = out.entry_lib_archive[dep_idx];
        hash_combiner.add(dep_hash);
    }

    let link_base_hash = hash_combiner.combine();

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

        let obj_hash = mix_hashes(basic_hash, entry_pcm_hash);

        test_object_hashes.push(obj_hash);

        let exe_hash = mix_hashes(obj_hash, link_base_hash);

        test_exe_hashes.push(exe_hash);

        let stamp_hash = mix_hashes(exe_hash, hash_bytes(b"stamp"));

        test_stamp_hashes.push(stamp_hash);
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

        let obj_hash = mix_hashes(basic_hash, entry_pcm_hash);

        bin_object_hashes.push(obj_hash);

        let exe_hash = mix_hashes(obj_hash, link_base_hash);

        bin_exe_hashes.push(exe_hash);
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
    let base_hash = construct_base_deep_hash(config, info.self_exe_hash);

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

fn collect_lib_path_list(
    config: &Config,
    interner: &mut Interner,
    info: &BuildInfo,
    entry_index: usize,
) -> &'static [&'static str] {
    let mut path_list: Vec<&'static str> = Vec::new();

    let self_name_idx = info.name[entry_index];

    let mut processed: HashSet<Str> = HashSet::default();
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

pub fn plan(
    config: &Config,
    subcommand: &Subcommand,
    interner: &mut Interner,
    info: &BuildInfo,
    cache_state: &mut CacheState,
    timestamp: u64,
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
        // collect the library path list for doing static linking.
        let lib_path_list = collect_lib_path_list(config, interner, info, eidx);

        let lib_path_indices: Vec<Str> = lib_path_list.iter().map(|p| interner.insert(p)).collect();

        let mut params = Params {
            config,
            subcommand,
            interner,
            info,
            cache_state,
            entry_index: eidx,
            deep_hashes: &deep_hashes,
            tasks: &mut tasks,
            timestamp,
        };

        plan_module_build(params.reborrow());
        plan_lib_build(params.reborrow());

        plan_test_build(params.reborrow(), lib_path_list, &lib_path_indices);
        plan_bin_build(params.reborrow(), lib_path_list, &lib_path_indices);
    }

    tasks
}
