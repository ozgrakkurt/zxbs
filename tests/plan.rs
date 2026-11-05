#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use build::Task;
use command::Command;
use interner::Interner;
use plan::make_module_artifact_name;
use zxbs::{build, command, config, interner, plan, scan};

#[test]
fn test_empty() {
    let folder_name: u128 = rand::random();
    let build_path = format!("/tmp/{}/build", folder_name);
    let src_path = format!("/tmp/{}/src", folder_name);
    std::fs::create_dir_all(&build_path).unwrap();
    std::fs::create_dir_all(&src_path).unwrap();

    let mut flags = HashMap::new();
    flags.insert("--build-dir".to_owned(), build_path.clone());
    flags.insert("--src-dir".to_owned(), src_path.clone());
    let cfg = config::Config::parse_config(flags);

    let mut interner = interner::Interner::new();

    let info = scan::BuildInfo::default();

    let tasks = plan::plan(&cfg, &config::Subcommand::TestAll, &mut interner, &info);

    assert!(tasks.is_empty());

    std::fs::remove_dir_all(format!("/tmp/{}", folder_name)).unwrap();
}

fn check_run_bin_task(
    task: &Task,
    interner: &mut Interner,
    build_path: &str,
    entry_name: &str,
    bin_name: &str,
    is_test: bool,
) {
    let folder_name = if is_test { "test" } else { "bin" };

    assert_eq!(
        &task.inputs,
        &[interner.insert(&format!(
            "{}/{}/{}/{}",
            build_path, entry_name, folder_name, bin_name
        ))]
    );

    assert_eq!(
        &task.outputs,
        &[interner.insert(&format!(
            "{}/{}/{}/stamp/{}",
            build_path, entry_name, folder_name, bin_name,
        ))],
    );

    match &task.command {
        Some(Command::RunBinary(rb)) => {
            assert_eq!(
                rb.path,
                format!("{}/{}/{}/{}", build_path, entry_name, folder_name, bin_name)
            );
        }
        _ => unreachable!(),
    }
}

fn check_link_bin_task(
    task: &Task,
    interner: &mut Interner,
    build_path: &str,
    entry_name: &str,
    bin_name: &str,
    is_test: bool,
    dep_names: &[&str],
) {
    let folder_name = if is_test { "test" } else { "bin" };

    let mut inputs = vec![interner.insert(&format!(
        "{}/{}/lib/lib{}.a",
        build_path, entry_name, entry_name,
    ))];
    let mut expected_deps = vec![interner.get(inputs[0]).to_owned()];
    for dn in dep_names.iter() {
        inputs.push(interner.insert(&format!("{}/{}/lib/lib{}.a", build_path, dn, dn)));
        expected_deps.push(format!("{}/{}/lib/lib{}.a", build_path, dn, dn));
    }

    inputs.push(interner.insert(&format!(
        "{}/{}/{}/{}.o",
        build_path, entry_name, folder_name, bin_name
    )));

    let expected_deps: Vec<&str> = expected_deps.iter().map(|d| d.as_str()).collect();

    assert_eq!(&task.inputs, &inputs,);

    assert_eq!(
        &task.outputs,
        &[interner.insert(&format!(
            "{}/{}/{}/{}",
            build_path, entry_name, folder_name, bin_name
        ))],
    );

    match &task.command {
        Some(Command::LinkBinary(lb)) => {
            assert_eq!(lb.cxx, "clang++");
            assert_eq!(
                lb.output,
                format!("{}/{}/{}/{}", build_path, entry_name, folder_name, bin_name)
            );
            assert_eq!(
                lb.object,
                format!(
                    "{}/{}/{}/{}.o",
                    build_path, entry_name, folder_name, bin_name
                )
            );
            assert_eq!(lb.deps, expected_deps.as_slice());
        }
        _ => unreachable!(),
    }
}

fn check_build_bin_task(
    task: &Task,
    interner: &mut Interner,
    build_path: &str,
    src_path: &str,
    entry_name: &str,
    bin_name: &str,
    is_test: bool,
) {
    let folder_name = if is_test { "test" } else { "bin" };

    assert_eq!(
        &task.inputs,
        &[interner.insert(&format!("{}/mod/{}.pcm", build_path, entry_name)),],
    );

    assert_eq!(
        &task.outputs,
        &[interner.insert(&format!(
            "{}/{}/{}/{}.o",
            build_path, entry_name, folder_name, bin_name,
        ))],
    );

    match &task.command {
        Some(Command::CompileToObject(co)) => {
            assert_eq!(co.cxx, "clang++");
            assert_eq!(
                co.input,
                format!(
                    "{}/{}/{}/{}.cc",
                    src_path, entry_name, folder_name, bin_name
                ),
            );
            assert_eq!(
                co.output,
                format!(
                    "{}/{}/{}/{}.o",
                    build_path, entry_name, folder_name, bin_name
                ),
            );
        }
        _ => unreachable!(),
    }
}

fn check_lib_archive_task(
    task: &Task,
    interner: &mut Interner,
    build_path: &str,
    entry_name: &str,
    mod_part_names: &[&str],
    lib_part_names: &[&str],
) {
    let mut inputs = Vec::new();
    for pn in mod_part_names.iter() {
        inputs.push(interner.insert(&format!(
            "{}/mod/{}",
            build_path,
            make_module_artifact_name(entry_name, pn, "o"),
        )));
    }
    for ln in lib_part_names.iter() {
        inputs.push(interner.insert(&format!("{}/{}/lib/{}.o", build_path, entry_name, ln,)));
    }

    assert_eq!(&task.inputs, &inputs);

    assert_eq!(
        &task.outputs,
        &[interner.insert(&format!(
            "{}/{}/lib/lib{}.a",
            build_path, entry_name, entry_name
        )),]
    );

    let expected_input_names: Vec<&'static str> = inputs.iter().map(|&x| interner.get(x)).collect();

    match &task.command {
        Some(Command::ArchiveLib(al)) => {
            assert_eq!(al.ar, "llvm-ar");
            assert_eq!(
                al.output,
                format!("{}/{}/lib/lib{}.a", build_path, entry_name, entry_name)
            );
            assert_eq!(al.inputs, expected_input_names);
        }
        _ => unreachable!(),
    }
}

fn check_lib_obj_task(
    task: &Task,
    interner: &mut Interner,
    build_path: &str,
    src_path: &str,
    entry_name: &str,
    obj_name: &str,
) {
    assert_eq!(
        &task.inputs,
        &[interner.insert(&format!("{}/mod/{}.pcm", build_path, entry_name,))],
    );

    assert_eq!(
        &task.outputs,
        &[interner.insert(&format!("{}/{}/lib/{}.o", build_path, entry_name, obj_name,))],
    );

    match &task.command {
        Some(Command::CompileToObject(co)) => {
            assert_eq!(co.cxx, "clang++");
            assert_eq!(
                co.input,
                format!("{}/{}/lib/{}.cc", src_path, entry_name, obj_name)
            );
            assert_eq!(
                co.output,
                format!("{}/{}/lib/{}.o", build_path, entry_name, obj_name),
            );
        }
        _ => unreachable!(),
    }
}

fn check_pcm_task(
    task: &Task,
    interner: &mut Interner,
    build_path: &str,
    src_path: &str,
    entry_name: &str,
    part_name: &str,
    local_deps: &[&str],
    ext_deps: &[&str],
) {
    let mut expected_inputs = Vec::new();
    for &extd in ext_deps.iter() {
        expected_inputs.push(interner.insert(&format!("{}/mod/{}.pcm", build_path, extd)));
    }
    for &ld in local_deps.iter() {
        expected_inputs.push(interner.insert(&format!(
            "{}/mod/{}",
            build_path,
            make_module_artifact_name(entry_name, ld, "pcm")
        )));
    }
    assert_eq!(&task.inputs, &expected_inputs);

    assert_eq!(
        &task.outputs,
        &[
            interner.insert(&format!(
                "{}/mod/{}",
                build_path,
                make_module_artifact_name(entry_name, part_name, "pcm")
            )),
            interner.insert(&format!(
                "{}/mod/{}",
                build_path,
                make_module_artifact_name(entry_name, part_name, "o")
            )),
        ]
    );

    match &task.command {
        Some(Command::CompileModule(cm)) => {
            assert_eq!(cm.cxx, "clang++");
            assert_eq!(
                cm.input,
                format!("{}/{}/mod/{}.cppm", src_path, entry_name, part_name)
            );
            assert_eq!(
                cm.object_output,
                format!(
                    "{}/mod/{}",
                    build_path,
                    make_module_artifact_name(entry_name, part_name, "o")
                )
            );
            assert_eq!(
                cm.pcm_output,
                format!(
                    "{}/mod/{}",
                    build_path,
                    make_module_artifact_name(entry_name, part_name, "pcm")
                )
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn test_simple() {
    let folder_name: u128 = rand::random();
    let build_path = format!("/tmp/{}/build", folder_name);
    let src_path = format!("/tmp/{}/src", folder_name);
    std::fs::create_dir_all(&build_path).unwrap();
    std::fs::create_dir_all(&src_path).unwrap();

    let mut flags = HashMap::new();
    flags.insert("--build-dir".to_owned(), build_path.clone());
    flags.insert("--src-dir".to_owned(), src_path.clone());
    let cfg = config::Config::parse_config(flags);

    let mut interner = interner::Interner::new();

    let mut info = scan::BuildInfo::default();

    let baselib = interner.insert("base");
    let iolib = interner.insert("io");
    let myapp = interner.insert("myapp");

    info.map.insert(baselib, 0);
    info.name.push(baselib);
    info.module.push(vec![
        interner.insert("base"),
        interner.insert("vec"),
        interner.insert("start"),
        interner.insert("arith"),
        interner.insert("alloc"),
    ]);
    #[rustfmt::skip]
    info.mod_dep_matrix.push(vec![
        false, true, true, true, true,
        false, false, false, true, true,
        false, false, false, false, false,
        false, false, true, false, false,
        false, false, true, true, false,
    ]);
    info.lib
        .push(vec![interner.insert("memcmp"), interner.insert("memcpy")]);
    info.bin.push(vec![interner.insert("memcpy_bench")]);
    info.test.push(vec![
        interner.insert("memcmp"),
        interner.insert("vec"),
        interner.insert("arith"),
    ]);
    info.deps.push(vec![]);

    info.map.insert(iolib, 1);
    info.name.push(iolib);
    info.module.push(vec![
        interner.insert("io"),
        interner.insert("executor"),
        interner.insert("fs"),
        interner.insert("net"),
        interner.insert("base"),
    ]);
    #[rustfmt::skip]
    info.mod_dep_matrix.push(vec![
        false, true, true, true, true,
        false, false, false, false, false,
        false, true, false, false, false,
        false, true, false, false, false,
        false, false, false, false, false,
    ]);
    info.lib.push(vec![]);
    info.bin.push(vec![interner.insert("direct_io_example")]);
    info.test.push(vec![
        interner.insert("executor_ordering"),
        interner.insert("fs"),
        interner.insert("net"),
    ]);
    info.deps.push(vec![baselib]);

    info.map.insert(myapp, 2);
    info.name.push(myapp);
    info.module.push(vec![interner.insert("myapp")]);
    info.mod_dep_matrix.push(vec![false]);
    info.lib.push(vec![]);
    info.bin.push(vec![myapp]);
    info.test.push(vec![]);
    info.deps.push(vec![baselib, iolib]);

    let tasks = plan::plan(&cfg, &config::Subcommand::TestAll, &mut interner, &info);

    check_pcm_task(
        &tasks[0],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "base",
        &["vec", "start", "arith", "alloc"],
        &[],
    );
    check_pcm_task(
        &tasks[1],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "vec",
        &["arith", "alloc"],
        &[],
    );
    check_pcm_task(
        &tasks[2],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "start",
        &[],
        &[],
    );
    check_pcm_task(
        &tasks[3],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "arith",
        &["start"],
        &[],
    );
    check_pcm_task(
        &tasks[4],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "alloc",
        &["start", "arith"],
        &[],
    );
    check_lib_obj_task(
        &tasks[5],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "memcmp",
    );
    check_lib_obj_task(
        &tasks[6],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "memcpy",
    );
    check_lib_archive_task(
        &tasks[7],
        &mut interner,
        &build_path,
        "base",
        &["base", "vec", "start", "arith", "alloc"],
        &["memcmp", "memcpy"],
    );
    check_build_bin_task(
        &tasks[8],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "memcmp",
        true,
    );
    check_link_bin_task(
        &tasks[9],
        &mut interner,
        &build_path,
        "base",
        "memcmp",
        true,
        &[],
    );
    check_run_bin_task(
        &tasks[10],
        &mut interner,
        &build_path,
        "base",
        "memcmp",
        true,
    );
    check_build_bin_task(
        &tasks[11],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "vec",
        true,
    );
    check_link_bin_task(
        &tasks[12],
        &mut interner,
        &build_path,
        "base",
        "vec",
        true,
        &[],
    );
    check_run_bin_task(&tasks[13], &mut interner, &build_path, "base", "vec", true);
    check_build_bin_task(
        &tasks[14],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "arith",
        true,
    );
    check_link_bin_task(
        &tasks[15],
        &mut interner,
        &build_path,
        "base",
        "arith",
        true,
        &[],
    );
    check_run_bin_task(
        &tasks[16],
        &mut interner,
        &build_path,
        "base",
        "arith",
        true,
    );
    check_build_bin_task(
        &tasks[17],
        &mut interner,
        &build_path,
        &src_path,
        "base",
        "memcpy_bench",
        false,
    );
    check_link_bin_task(
        &tasks[18],
        &mut interner,
        &build_path,
        "base",
        "memcpy_bench",
        false,
        &[],
    );
    check_pcm_task(
        &tasks[19],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "io",
        &["executor", "fs", "net", "base"],
        &["base"],
    );
    check_pcm_task(
        &tasks[20],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "executor",
        &[],
        &["base"],
    );
    check_pcm_task(
        &tasks[21],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "fs",
        &["executor"],
        &["base"],
    );
    check_pcm_task(
        &tasks[22],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "net",
        &["executor"],
        &["base"],
    );
    check_pcm_task(
        &tasks[23],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "base",
        &[],
        &["base"],
    );
    check_lib_archive_task(
        &tasks[24],
        &mut interner,
        &build_path,
        "io",
        &["io", "executor", "fs", "net", "base"],
        &[],
    );
    check_build_bin_task(
        &tasks[25],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "executor_ordering",
        true,
    );
    check_link_bin_task(
        &tasks[26],
        &mut interner,
        &build_path,
        "io",
        "executor_ordering",
        true,
        &["base"],
    );
    check_run_bin_task(
        &tasks[27],
        &mut interner,
        &build_path,
        "io",
        "executor_ordering",
        true,
    );
    check_build_bin_task(
        &tasks[28],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "fs",
        true,
    );
    check_link_bin_task(
        &tasks[29],
        &mut interner,
        &build_path,
        "io",
        "fs",
        true,
        &["base"],
    );
    check_run_bin_task(&tasks[30], &mut interner, &build_path, "io", "fs", true);
    check_build_bin_task(
        &tasks[31],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "net",
        true,
    );
    check_link_bin_task(
        &tasks[32],
        &mut interner,
        &build_path,
        "io",
        "net",
        true,
        &["base"],
    );
    check_run_bin_task(&tasks[33], &mut interner, &build_path, "io", "net", true);
    check_build_bin_task(
        &tasks[34],
        &mut interner,
        &build_path,
        &src_path,
        "io",
        "direct_io_example",
        false,
    );
    check_link_bin_task(
        &tasks[35],
        &mut interner,
        &build_path,
        "io",
        "direct_io_example",
        false,
        &["base"],
    );
    check_pcm_task(
        &tasks[36],
        &mut interner,
        &build_path,
        &src_path,
        "myapp",
        "myapp",
        &[],
        &["base", "io"],
    );
    check_lib_archive_task(
        &tasks[37],
        &mut interner,
        &build_path,
        "myapp",
        &["myapp"],
        &[],
    );
    check_build_bin_task(
        &tasks[38],
        &mut interner,
        &build_path,
        &src_path,
        "myapp",
        "myapp",
        false,
    );
    check_link_bin_task(
        &tasks[39],
        &mut interner,
        &build_path,
        "myapp",
        "myapp",
        false,
        &["io", "base"],
    );

    assert_eq!(tasks.len(), 40);

    std::fs::remove_dir_all(format!("/tmp/{}", folder_name)).unwrap();
}
