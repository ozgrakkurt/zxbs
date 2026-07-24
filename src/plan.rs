use crate::build::{Build, Executor, Task, TaskIndex};
use crate::command::{
    ArchiveLib, Command, CompileToObject, LinkBinary, PrecompileToPcm, RunBinary,
};
use crate::config::Config;
use crate::interner::{Interner, Str};
use crate::scan::BuildInfo;

fn make_module_artifact_name(root_name: &str, part_name: &str, extension: &str) -> String {
    if root_name == part_name {
        format!("{}.{}", root_name, extension)
    } else {
        format!("{}-{}.{}", root_name, part_name, extension)
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

    let mut ext_pcm_paths: Vec<PathIndex> = Vec::with_capacity(entry_deps.len());
    let mod_build_path = config.build_path.join("mod");
    std::fs::create_dir_all(&mod_build_path).unwrap();
    for &dep_name in entry_deps.iter() {
        let dep_name = name_interner.get_name(dep_name);
        let dep_pcm_path = mod_build_path.join(format!("{}.pcm", dep_name));
        let dep_pcm_path = path_interner.insert_path(&dep_pcm_path);
        ext_pcm_paths.push(dep_pcm_path);
    }

    let entry_name = name_interner.get_name(info.name[entry_index]);

    let mut local_pcm_paths: Vec<PathIndex> = Vec::with_capacity(n_mod_parts);
    let mut local_object_paths: Vec<PathIndex> = Vec::with_capacity(n_mod_parts);
    for &part_name in mod_parts.iter() {
        let part_name = name_interner.get_name(part_name);
        let part_pcm_path =
            mod_build_path.join(make_module_artifact_name(entry_name, part_name, "pcm"));
        let part_object_path =
            mod_build_path.join(make_module_artifact_name(entry_name, part_name, "o"));
        let part_pcm_path = path_interner.insert_path(&part_pcm_path);
        let part_object_path = path_interner.insert_path(&part_object_path);
        local_pcm_paths.push(part_pcm_path);
        local_object_paths.push(part_object_path);
    }

    let mod_src_path = config
        .src_path
        .join(name_interner.get_name(info.name[entry_index]))
        .join("mod");

    for (part_idx, &part) in mod_parts.iter().enumerate() {
        let task_index = TaskIndex::from_usize(tasks.len());

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

        let output_indices = vec![local_pcm_paths[part_idx]];

        let cppm_path = mod_src_path.join(format!("{}.cppm", name_interner.get_name(part)));
        let prebuilt_module_path_flag =
            format!("-fprebuilt-module-path={}", mod_build_path.display());

        let mut build_pcm_command = smol::process::Command::new(config.cxx);
        build_pcm_command
            .args(config.cflags)
            .arg(cppm_path.display().to_string())
            .arg("--precompile")
            .arg(prebuilt_module_path_flag.clone())
            .arg("-o")
            .arg(
                path_interner
                    .get_path(local_pcm_paths[part_idx])
                    .display()
                    .to_string(),
            );

        tasks.push(Task {
            input_indices,
            output_indices,
            command: Some(Box::pin(async move {
                let output = build_pcm_command.output().await.unwrap();
                if !output.status.success() {
                    panic!(
                        "Failed precompile:\n{}",
                        str::from_utf8(&output.stderr).unwrap()
                    );
                }
                task_index
            })),
        });

        let input_indices = vec![local_pcm_paths[part_idx]];
        let output_indices = vec![local_object_paths[part_idx]];

        let task_index = TaskIndex::from_usize(tasks.len());

        let mut build_obj_command = smol::process::Command::new(config.cxx);
        build_obj_command
            .args(config.cflags)
            .arg(
                path_interner
                    .get_path(local_pcm_paths[part_idx])
                    .display()
                    .to_string(),
            )
            .arg(prebuilt_module_path_flag.clone())
            .arg("-c")
            .arg("-o")
            .arg(
                path_interner
                    .get_path(local_object_paths[part_idx])
                    .display()
                    .to_string(),
            );

        tasks.push(Task {
            input_indices,
            output_indices,
            command: Some(Box::pin(async move {
                let output = build_obj_command.output().await.unwrap();
                if !output.status.success() {
                    panic!(
                        "Failed to compile pcm into object file:\n{}",
                        str::from_utf8(&output.stderr).unwrap()
                    );
                }
                task_index
            })),
        });
    }
}

fn plan_lib_build(
    config: &Config,
    name_interner: &NameInterner,
    path_interner: &mut PathInterner,
    tasks: &mut Vec<Task>,
    info: &BuildInfo,
    entry_index: usize,
) {
    // build object files (both .cc and .s)
    // archive into .a file

    let entry_name = name_interner.get_name(info.name[entry_index]);

    let parts = &info.lib[entry_index];
    let src_path = config.src_path.join(entry_name).join("lib");
    let build_path = config.build_path.join(entry_name).join("lib");

    std::fs::create_dir_all(&build_path).unwrap();

    let root_pcm_path = config
        .build_path
        .join("mod")
        .join(format!("{}.pcm", entry_name));
    let root_pcm_path_index = path_interner.insert_path(&root_pcm_path);

    let mod_build_path = config.build_path.join("mod");
    let prebuilt_module_path_flag = format!("-fprebuilt-module-path={}", mod_build_path.display());

    let mod_parts = &info.module[entry_index];

    let mut obj_path_indices: Vec<PathIndex> = Vec::with_capacity(parts.len() + mod_parts.len());
    let mut obj_paths: Vec<String> = Vec::with_capacity(parts.len() + mod_parts.len());

    for &mod_part_name_idx in mod_parts.iter() {
        let part_name = name_interner.get_name(mod_part_name_idx);
        let mod_part_obj_path =
            mod_build_path.join(make_module_artifact_name(entry_name, part_name, "o"));
        let path_idx = path_interner.insert_path(&mod_part_obj_path);
        obj_path_indices.push(path_idx);
        obj_paths.push(mod_part_obj_path.display().to_string());
    }

    for &part_name_idx in parts.iter() {
        let part_name = name_interner.get_name(part_name_idx);
        let cc_path = src_path.join(format!("{}.cc", part_name));
        let obj_path = build_path.join(format!("{}.o", part_name));
        let obj_path_index = path_interner.insert_path(&obj_path);

        obj_path_indices.push(obj_path_index);
        obj_paths.push(obj_path.display().to_string());

        let input_indices = vec![root_pcm_path_index];
        let output_indices = vec![obj_path_index];

        let mut build_obj_command = smol::process::Command::new(config.cxx);
        build_obj_command
            .args(config.cflags)
            .arg(cc_path.display().to_string())
            .arg(prebuilt_module_path_flag.clone())
            .arg("-c")
            .arg("-o")
            .arg(obj_path.display().to_string());

        let task_index = TaskIndex::from_usize(tasks.len());
        tasks.push(Task {
            input_indices,
            output_indices,
            command: Some(Box::pin(async move {
                let output = build_obj_command.output().await.unwrap();
                if !output.status.success() {
                    panic!(
                        "Failed to compile library cc file into object file:\n{}",
                        str::from_utf8(&output.stderr).unwrap()
                    );
                }
                task_index
            })),
        })
    }

    let lib_output_path = build_path.join(format!("lib{}.a", entry_name));
    let lib_output_path_idx = path_interner.insert_path(&lib_output_path);

    let input_indices = obj_path_indices;
    let output_indices = vec![lib_output_path_idx];

    let mut archive_lib_command = smol::process::Command::new(config.ar);
    archive_lib_command
        .arg("crs")
        .arg(lib_output_path.display().to_string())
        .args(obj_paths);

    let task_index = TaskIndex::from_usize(tasks.len());
    tasks.push(Task {
        input_indices,
        output_indices,
        command: Some(Box::pin(async move {
            let output = archive_lib_command.output().await.unwrap();
            if !output.status.success() {
                panic!(
                    "Failed to archive library:\n{}",
                    str::from_utf8(&output.stderr).unwrap(),
                );
            }
            task_index
        })),
    });
}

fn plan_bin_build(
    config: &Config,
    name_interner: &NameInterner,
    path_interner: &mut PathInterner,
    tasks: &mut Vec<Task>,
    info: &BuildInfo,
    entry_index: usize,
    lib_paths: &'static [&'static str],
    lib_path_indices: &[PathIndex],
    is_test: bool,
) {
    // depend on self lib (.a)
    // compile cc to .o
    // do linking

    let artifact_kind_name = if is_test { "test" } else { "bin" };

    let entry_name = name_interner.get_name(info.name[entry_index]);

    let self_pcm_path = config
        .build_path
        .join("mod")
        .join(format!("{}.pcm", entry_name));
    let self_pcm_path_idx = path_interner.insert_path(&self_pcm_path);

    let bins = if is_test {
        &info.test[entry_index]
    } else {
        &info.bin[entry_index]
    };
    let build_path = config.build_path.join(entry_name).join(artifact_kind_name);
    let src_path = config.src_path.join(entry_name).join(artifact_kind_name);

    let mod_build_path = config.build_path.join("mod");

    std::fs::create_dir_all(&build_path).unwrap();

    let prebuilt_module_path_flag = format!("-fprebuilt-module-path={}", mod_build_path.display());

    for &bin_name_idx in bins.iter() {
        // emit obj compilation task
        // emit linking task

        let bin_name = name_interner.get_name(bin_name_idx);

        let cc_path = src_path.join(format!("{}.cc", bin_name));

        let obj_path = build_path.join(format!("{}.o", bin_name));
        let obj_path_index = path_interner.insert_path(&obj_path);
        let exe_path = build_path.join(bin_name);
        let exe_path_index = path_interner.insert_path(&exe_path);

        let input_indices = vec![self_pcm_path_idx];
        let output_indices = vec![obj_path_index];

        let mut obj_command = smol::process::Command::new(config.cxx);
        obj_command
            .args(config.cflags)
            .arg(cc_path.display().to_string())
            .arg(prebuilt_module_path_flag.clone())
            .arg("-c")
            .arg("-o")
            .arg(obj_path.display().to_string());

        let task_index = TaskIndex::from_usize(tasks.len());
        tasks.push(Task {
            input_indices,
            output_indices,
            command: Some(Box::pin(async move {
                let output = obj_command.output().await.unwrap();
                if !output.status.success() {
                    panic!(
                        "Failed to compile main object file of binary:\n{}",
                        str::from_utf8(&output.stderr).unwrap(),
                    );
                }
                task_index
            })),
        });

        let mut input_indices = lib_path_indices.to_vec();
        input_indices.push(obj_path_index);
        let output_indices = vec![exe_path_index];

        let mut link_command = smol::process::Command::new(config.cxx);
        link_command
            .args(config.ldflags)
            .arg("-o")
            .arg(exe_path.display().to_string())
            .arg(obj_path.display().to_string())
            .args(lib_paths);

        let task_index = TaskIndex::from_usize(tasks.len());
        tasks.push(Task {
            input_indices,
            output_indices,
            command: Some(Box::pin(async move {
                let output = link_command.output().await.unwrap();
                if !output.status.success() {
                    panic!(
                        "Failed to link binary:\n{}",
                        str::from_utf8(&output.stderr).unwrap(),
                    );
                }
                task_index
            })),
        });

        if is_test {
            let input_indices = vec![exe_path_index];
            let output_indices = Vec::new();

            let mut test_command = smol::process::Command::new(exe_path.display().to_string());

            let task_index = TaskIndex::from_usize(tasks.len());
            tasks.push(Task {
                input_indices,
                output_indices,
                command: Some(Box::pin(async move {
                    let output = test_command.output().await.unwrap();
                    if !output.status.success() {
                        panic!(
                            "Failed to run test ({}):\n{}",
                            output.status,
                            str::from_utf8(&output.stderr).unwrap(),
                        );
                    }
                    task_index
                })),
            });
        }
    }
}

fn collect_lib_path_list(
    config: &Config,
    name_interner: &NameInterner,
    path_interner: &mut PathInterner,
    info: &BuildInfo,
    entry_index: usize,
) -> &'static [&'static str] {
    let mut path_list: Vec<&'static str> = Vec::new();

    let self_name_idx = info.name[entry_index];

    let mut processed: HashSet<NameIndex> = HashSet::new();
    let mut worklist: Vec<NameIndex> = vec![self_name_idx];
    while let Some(name_idx) = worklist.pop() {
        let pos_idx: usize = usize::try_from(*info.map.get(&name_idx).unwrap()).unwrap();

        let name = name_interner.get_name(name_idx);

        let lib_path = config
            .build_path
            .join(name)
            .join("lib")
            .join(format!("lib{}.a", name));
        let lib_path_idx = path_interner.insert_path(&lib_path);
        path_list.push(path_interner.get_path(lib_path_idx).to_str().unwrap());

        for &dep_name_idx in info.deps[pos_idx].iter() {
            // check deps don't cycle back to the initial name
            assert_ne!(dep_name_idx, self_name_idx);
            // deduplicate using the set, so we don't reprocess same names again
            if processed.insert(dep_name_idx) {
                worklist.push(dep_name_idx);
            }
        }
    }

    path_list.leak()
}

fn plan(
    config: &Config,
    name_interner: &NameInterner,
    path_interner: &mut PathInterner,
    info: &BuildInfo,
) -> Vec<Task> {
    assert_eq!(info.name.len(), info.mod_dep_matrix.len());
    assert_eq!(info.name.len(), info.module.len());
    assert_eq!(info.name.len(), info.lib.len());
    assert_eq!(info.name.len(), info.bin.len());
    assert_eq!(info.name.len(), info.test.len());
    assert_eq!(info.name.len(), info.deps.len());

    let mut tasks = Vec::new();

    for eidx in 0..info.name.len() {
        plan_module_build(config, name_interner, path_interner, &mut tasks, info, eidx);
        plan_lib_build(config, name_interner, path_interner, &mut tasks, info, eidx);

        // collect the library path list for doing static linking.
        let lib_path_list = collect_lib_path_list(config, name_interner, path_interner, info, eidx);

        let lib_path_indices: Vec<PathIndex> = lib_path_list
            .iter()
            .map(|p| path_interner.insert_path(Path::new(p)))
            .collect();

        for is_test in [true, false] {
            plan_bin_build(
                config,
                name_interner,
                path_interner,
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
