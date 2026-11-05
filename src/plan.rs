use std::collections::HashSet;

use crate::build::Task;
use crate::command::{
    ArchiveLib, Command, CompileModule, CompileToObject, Format, LinkBinary, RunBinary,
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

// checks if given binary part should run according to the subcommand.
// Returns None if it shouldn't run.
// Returns Some(arguments) if it should run. The arguments are supposed to be passed as cli arguments into the binary.
pub fn should_run(
    subcommand: &Subcommand,
    is_test: bool,
    entry: &str,
    part: &str,
) -> Option<&'static [&'static str]> {
    if is_test {
        match subcommand {
            Subcommand::TestAll => Some(&[]),
            Subcommand::Test {
                entry_name,
                test_name,
            } => {
                if entry == entry_name && part == test_name {
                    Some(&[])
                } else {
                    None
                }
            }
            _ => None,
        }
    } else {
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
}

fn plan_module_build(
    config: &Config,
    interner: &mut Interner,
    tasks: &mut Vec<Task>,
    info: &BuildInfo,
    entry_index: usize,
) {
    let mod_parts = info.module[entry_index].as_slice();
    let n_mod_parts = mod_parts.len();

    let entry_deps = info.deps[entry_index].as_slice();

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

pub fn plan(
    config: &Config,
    subcommand: &Subcommand,
    interner: &mut Interner,
    info: &BuildInfo,
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

    let mut tasks = Vec::new();

    for eidx in 0..info.name.len() {
        plan_module_build(config, interner, &mut tasks, info, eidx);
        plan_lib_build(config, interner, &mut tasks, info, eidx);

        // collect the library path list for doing static linking.
        let lib_path_list = collect_lib_path_list(config, interner, info, eidx);

        let lib_path_indices: Vec<Str> = lib_path_list.iter().map(|p| interner.insert(p)).collect();

        for is_test in [true, false] {
            plan_bin_build(
                config,
                subcommand,
                interner,
                &mut tasks,
                info,
                eidx,
                lib_path_list,
                &lib_path_indices,
                is_test,
            );
        }
    }

    tasks
}
