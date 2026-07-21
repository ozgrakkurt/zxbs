#![allow(clippy::single_match)]
#![allow(clippy::wrong_self_convention)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::too_many_arguments)]

use smol::future::Boxed as BoxedFuture;
use smol::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug)]
struct Config {
    cxx: &'static str,
    ar: &'static str,
    clang_scan_deps: &'static str,
    cflags: &'static [&'static str],
    ldflags: &'static [&'static str],
    build_path: &'static Path,
    src_path: &'static Path,
}

fn get_bool_arg(args: &HashMap<String, String>, name: &str, default: bool) -> bool {
    let positive = format!("--{}", name);
    let negative = format!("--no-{}", name);

    match args.get(&positive) {
        Some(v) => {
            assert_eq!(v.as_str(), "");
            assert!(args.get(&negative).is_none());
            return true;
        }
        None => {}
    }

    match args.get(&negative) {
        Some(v) => {
            assert_eq!(v.as_str(), "");
            return false;
        }
        None => {}
    }

    default
}

fn make_path_absolute(cwd: &Path, path: &Path) -> PathBuf {
    if !path.is_absolute() {
        cwd.join(path)
    } else {
        path.to_owned()
    }
}

struct FlagCfg {
    opt_level: String,
    dev_mode: bool,
    debug_info: bool,
    fuzz: bool,
}

fn make_cflags(cfg: &FlagCfg) -> Vec<&'static str> {
    let mut cflags = vec![
        // Warnings.
        "-Wall",
        "-Wextra",
        "-Wconversion",
        "-Wsign-conversion",
        "-Wmissing-field-initializers",
        "-Wshadow",
        "-Wimplicit-fallthrough",
        // We only use C style structs so this is not an issue. This can cause bugs
        // if we ever use types with constructors?
        "-Wno-reorder-init-list",
        // This causes annoying warnings with templated code.
        // Could try to enable it and then check if there are any interesting catches later.
        "-Wno-tautological-compare",
        // warn on variable length array usage
        "-Wvla",
        // these don't seem to catch much but enable anyways
        "-Wpointer-bool-conversion",
        "-Wbool-operation",
        // make sure switch on enum has a default
        "-Wswitch-default",
        // Treat warnings as errors.
        "-Werror",
        // Target micro architecture.
        "-march=native",
        // Use cpp standard.
        "-std=c++23",
        // we don't use libc but this might effect some compiler builtins?
        "-fno-math-errno",
        // this caused compilation errors so is disabled. could be an idea to re-enable it
        "-fno-stack-protector",
        // enable linker garbage collection
        "-ffunction-sections",
        "-fdata-sections",
        // disable strict aliasing
        "-fno-strict-aliasing",
        // disable undefined behavior on signed integer overflow
        "-fno-strict-overflow",
        // signed integer overflow is defined to wrap.
        "-fwrapv",
        // Don't delete `ptr == nullptr` kind of checks. And don't interpret passing nullptr on some places as
        // undefined behavior.
        "-fno-delete-null-pointer-checks",
        // disable some c++ features
        "-fno-exceptions",
        "-fno-rtti",
        "-nostdlib++",
        "-fno-threadsafe-statics",
        "-fomit-frame-pointer",
        "-momit-leaf-frame-pointer",
        // disable libc
        "-nostdlib",
        "-nostdinc",
        "--target=x86_64-linux-none",
    ];

    cflags.push(format!("-O{}", cfg.opt_level).leak());

    if cfg.debug_info {
        cflags.push("-g1");
        cflags.push("-gdwarf64");
        cflags.push("-gdwarf-5");
        cflags.push("-funwind-tables");
        cflags.push("-fasynchronous-unwind-tables");
        cflags.push("-DZX_DEBUG_INFO");
    } else {
        cflags.push("-fno-unwind-tables");
        cflags.push("-fno-asynchronous-unwind-tables");
    }

    if cfg.dev_mode {
        cflags.push("-DZX_DEV");
    }

    if cfg.fuzz {
        cflags.push("-DZX_FUZZ");
        cflags.push("-fsanitize-coverage=trace-pc-guard");
    }

    cflags
}

fn make_ldflags(cfg: &FlagCfg) -> Vec<&'static str> {
    let mut ldflags = vec![
        "-march=native",
        // use llvm linker
        "-fuse-ld=lld",
        // don't allow undefined references
        "-Wl,-z,defs",
        // remove unused sections
        "-Wl,--gc-sections",
        // disable some c++ features
        "-fno-exceptions",
        "-fno-rtti",
        "-nostdlib++",
        "-fno-threadsafe-statics",
        "-fomit-frame-pointer",
        "-momit-leaf-frame-pointer",
        "-static",
        // disable libc
        "-nostdlib",
        "-nostdinc",
        "--target=x86_64-linux-none",
    ];

    ldflags.push(format!("-O{}", cfg.opt_level).leak());

    if cfg.debug_info {
        ldflags.push("-g1");
        ldflags.push("-gdwarf64");
        ldflags.push("-gdwarf-5");
        ldflags.push("-funwind-tables");
        ldflags.push("-fasynchronous-unwind-tables");
        ldflags.push("-DZX_DEBUG_INFO");
    } else {
        ldflags.push("-fno-unwind-tables");
        ldflags.push("-fno-asynchronous-unwind-tables");
        ldflags.push("-Wl,-s");
    }

    ldflags
}

fn leak_str(s: &String) -> &'static str {
    s.clone().leak()
}

fn parse_args() -> Config {
    let mut args = HashMap::<String, String>::new();

    for arg in std::env::args().skip(1) {
        let parts = arg.split('=').collect::<Vec<&str>>();

        match parts.len() {
            1 => {
                if args.insert(arg.clone(), "".to_owned()).is_some() {
                    panic!("Duplicate argument: {}", arg);
                }
            }
            2 => {
                if args
                    .insert(parts[0].to_owned(), parts[1].to_owned())
                    .is_some()
                {
                    panic!("Duplicate argument: {}", parts[0]);
                }
            }
            _ => {
                panic!("Invalid CLI argument: {}", arg);
            }
        }
    }

    let cxx = args.get("--cxx").map(leak_str).unwrap_or("clang++");
    let ar = args.get("--ar").map(leak_str).unwrap_or("llvm-ar");
    let clang_scan_deps = args
        .get("--clang-scan-deps")
        .map(leak_str)
        .unwrap_or("clang-scan-deps");
    let opt_level = args.get("--opt-level").map(leak_str).unwrap_or("2");
    let dev_mode = get_bool_arg(&args, "dev-mode", true);
    let debug_info = get_bool_arg(&args, "debug-info", true);
    let fuzz = get_bool_arg(&args, "fuzz", false);
    let src_dir = args.get("--src-dir").map(leak_str).unwrap_or("src");
    let build_dir: &'static str = args.get("--build-dir").map(leak_str).unwrap_or("build");

    let current_path = std::env::current_dir().unwrap();
    let src_path = make_path_absolute(&current_path, Path::new(src_dir)).leak();
    let build_path = make_path_absolute(&current_path, Path::new(build_dir)).leak();

    let flag_cfg = FlagCfg {
        opt_level: opt_level.to_owned(),
        dev_mode,
        debug_info,
        fuzz,
    };
    let cflags = make_cflags(&flag_cfg).leak();
    let ldflags = make_ldflags(&flag_cfg).leak();

    Config {
        cxx: cxx.to_owned().leak(),
        ar: ar.to_owned().leak(),
        clang_scan_deps: clang_scan_deps.to_owned().leak(),
        build_path,
        src_path,
        cflags,
        ldflags,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct PathIndex(u32);

impl PathIndex {
    fn from_usize(v: usize) -> PathIndex {
        PathIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct NameIndex(u32);

impl NameIndex {
    fn from_usize(v: usize) -> NameIndex {
        NameIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct TaskIndex(u32);

impl TaskIndex {
    fn from_usize(v: usize) -> TaskIndex {
        TaskIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

#[derive(Default)]
struct NameInterner {
    map: HashMap<&'static str, NameIndex>,
    names: Vec<&'static str>,
}

impl NameInterner {
    fn insert_name(&mut self, name: &str) -> NameIndex {
        match self.map.get(name) {
            Some(idx) => *idx,
            None => {
                let name = name.to_owned().leak();
                let idx = NameIndex::from_usize(self.names.len());
                self.names.push(name);
                self.map.insert(name, idx);
                idx
            }
        }
    }

    fn get_name(&self, idx: NameIndex) -> &'static str {
        self.names[idx.to_usize()]
    }

    // fn len(&self) -> usize {
    //     self.names.len()
    // }
}

#[derive(Default)]
struct PathInterner {
    map: HashMap<&'static str, PathIndex>,
    paths: Vec<&'static str>,
}

impl PathInterner {
    fn insert_path(&mut self, path: &Path) -> PathIndex {
        let path = path.to_str().unwrap();
        match self.map.get(path) {
            Some(idx) => *idx,
            None => {
                let path = path.to_owned().leak();
                let idx = PathIndex::from_usize(self.paths.len());
                self.paths.push(path);
                self.map.insert(path, idx);
                idx
            }
        }
    }

    fn get_path(&self, idx: PathIndex) -> &'static Path {
        Path::new(self.paths[idx.to_usize()])
    }

    fn len(&self) -> usize {
        self.paths.len()
    }
}

struct Task {
    input_indices: Vec<PathIndex>,
    output_indices: Vec<PathIndex>,
    command: Option<BoxedFuture<TaskIndex>>,
}

type Executor = smol::Executor<'static>;

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct Build {
    executor: &'static Executor,
    is_built: Vec<bool>,
    dependencies: Vec<Vec<PathIndex>>,
    dependants: Vec<Vec<TaskIndex>>,
    tasks: Vec<Task>,
    running: Vec<smol::Task<TaskIndex>>,
    backlog: VecDeque<TaskIndex>,
    concurrency: usize,
    done: bool,
}

impl Build {
    fn new(executor: &'static Executor, tasks: Vec<Task>, num_paths: usize) -> Build {
        let mut dependencies = Vec::with_capacity(tasks.len());

        let mut dependants = vec![Vec::new(); num_paths];

        let mut backlog = VecDeque::new();

        for (idx, task) in tasks.iter().enumerate() {
            let task_idx = TaskIndex::from_usize(idx);
            dependencies.push(task.input_indices.to_vec());
            for input_idx in task.input_indices.iter() {
                dependants[input_idx.to_usize()].push(task_idx);
            }

            if task.input_indices.is_empty() {
                backlog.push_back(task_idx);
            }
        }

        Build {
            executor,
            is_built: vec![false; tasks.len()],
            dependencies,
            dependants,
            tasks,
            running: Vec::new(),
            backlog,
            concurrency: num_cpus::get(),
            done: false,
        }
    }

    fn process_task_finished(&mut self, task_idx: TaskIndex) {
        let task = &self.tasks[task_idx.to_usize()];
        for out_idx in task.output_indices.iter() {
            assert!(!self.is_built[out_idx.to_usize()]);
            self.is_built[out_idx.to_usize()] = true;

            for &dependant_idx in self.dependants[out_idx.to_usize()].iter() {
                let deps: &mut Vec<PathIndex> = &mut self.dependencies[dependant_idx.to_usize()];
                let pos = deps.iter().position(|idx| idx.0 == out_idx.0).unwrap();
                deps.swap_remove(pos);

                if deps.is_empty() {
                    self.backlog.push_back(dependant_idx);
                }
            }
        }
    }
}

impl Future for Build {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let fut = self.get_mut();

        assert!(!fut.done);

        let mut idx = 0;
        while idx < fut.running.len() {
            let job = &mut fut.running[idx];
            match job.poll(cx) {
                Poll::Ready(task_idx) => {
                    fut.process_task_finished(task_idx);
                    std::mem::drop(fut.running.swap_remove(idx));
                }
                Poll::Pending => {
                    idx += 1;
                }
            }
        }

        while fut.running.len() < fut.concurrency {
            let task_idx = match fut.backlog.pop_front() {
                Some(task_idx) => task_idx,
                None => break,
            };

            let command = fut.tasks[task_idx.to_usize()].command.take().unwrap();
            let mut handle = fut.executor.spawn(command);

            match handle.poll(cx) {
                Poll::Ready(task_idx) => {
                    fut.process_task_finished(task_idx);
                }
                Poll::Pending => {
                    fut.running.push(handle);
                }
            }
        }

        if fut.running.is_empty() && fut.backlog.is_empty() {
            fut.done = true;
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

async fn scan_deps(config: &Config, path: &Path) -> Vec<String> {
    let mut cmd = smol::process::Command::new(config.clang_scan_deps);
    cmd.arg("-format=p1689")
        .arg("--")
        .arg(config.cxx)
        .args(config.cflags)
        .arg("-c")
        .arg(path);
    let output = cmd.output().await.unwrap();
    if !output.status.success() {
        panic!(
            "Failed clang_scan_deps:\n{}",
            str::from_utf8(&output.stderr).unwrap()
        );
    }
    let contents = output.stdout;

    let root = serde_json::from_slice::<serde_json::Value>(&contents).unwrap();
    let root = root.as_object().unwrap();

    let mut deps = Vec::new();

    for rule in root.get("rules").unwrap().as_array().unwrap().iter() {
        let rule = rule.as_object().unwrap();
        let reqs = match rule.get("requires") {
            Some(req) => req,
            None => continue,
        };
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

    deps
}

async fn read_deps_file(interner: &mut NameInterner, path: &Path) -> Vec<NameIndex> {
    let contents = String::from_utf8(smol::fs::read(path).await.unwrap()).unwrap();
    let mut indx = Vec::new();
    for line in contents.trim().split('\n') {
        let line = line.trim();
        if !line.is_empty() {
            indx.push(interner.insert_name(line));
        }
    }
    indx
}

#[derive(Default)]
struct BuildInfo {
    map: HashMap<NameIndex, u32>,
    name: Vec<NameIndex>,
    module: Vec<Vec<NameIndex>>,
    // matrix of mod_count * mod_count.
    // matrix[i * mod_count + j] is true if mod no i depends on mod no j.
    mod_dep_matrix: Vec<Vec<bool>>,
    lib: Vec<Vec<NameIndex>>,
    bin: Vec<Vec<NameIndex>>,
    test: Vec<Vec<NameIndex>>,
    deps: Vec<Vec<NameIndex>>,
}

fn check_name(name: &str) {
    for &byte in name.as_bytes() {
        let is_char = byte >= b'a' && byte <= b'z';
        let is_underscore = byte == b'_';
        assert!(is_char || is_underscore);
    }
}

fn parse_file_name<'name>(expected_extension: &str, file_name: &'name str) -> &'name str {
    let parts: Vec<_> = file_name.split(".").collect();
    assert_eq!(parts.len(), 2);
    let name = parts[0];
    let extension = parts[1];

    check_name(name);

    assert_eq!(expected_extension, extension);

    name
}

async fn scan_subfolder(
    expected_extension: &str,
    interner: &mut NameInterner,
    path: &Path,
) -> Vec<NameIndex> {
    let mut dir = smol::fs::read_dir(path).await.unwrap();
    let mut names = Vec::new();
    while let Some(file) = dir.next().await {
        let file = file.unwrap();
        let name =
            parse_file_name(expected_extension, file.file_name().to_str().unwrap()).to_owned();
        check_name(&name);
        let name_index = interner.insert_name(&name);
        names.push(name_index);
    }
    names
}

async fn scan(config: &Config, interner: &mut NameInterner) -> BuildInfo {
    let mut dir = smol::fs::read_dir(config.src_path).await.unwrap();

    let mut info = BuildInfo::default();

    let mut entry_idx: u32 = 0;
    while let Some(entry) = dir.next().await {
        let entry = entry.unwrap();
        let path = entry.path();
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_str().unwrap();
        check_name(entry_name);
        let entry_name: NameIndex = interner.insert_name(entry_name);
        assert!(info.map.insert(entry_name, entry_idx).is_none());

        info.name.push(entry_name);

        let deps = read_deps_file(interner, &path.join("deps")).await;

        let mod_parts = scan_subfolder("cppm", interner, &path.join("mod")).await;
        let n_mod_parts = mod_parts.len();

        // has to have root module
        assert!(mod_parts.contains(&entry_name));

        let mod_path = path.join("mod");

        let mut mod_dep_matrix = vec![false; n_mod_parts * n_mod_parts];
        for (part_pos, &part_name) in mod_parts.iter().enumerate() {
            let mod_dep_names = scan_deps(
                config,
                &mod_path.join(format!("{}.cppm", interner.get_name(part_name))),
            )
            .await;
            for mod_dep_name in mod_dep_names.iter() {
                if mod_dep_name.contains(':') {
                    let parts: Vec<_> = mod_dep_name.split(':').collect();
                    assert_eq!(parts.len(), 2);
                    check_name(parts[0]);
                    check_name(parts[1]);
                    let part0 = interner.insert_name(parts[0]);
                    let part1 = interner.insert_name(parts[1]);

                    assert_eq!(part0, entry_name);
                    assert_ne!(part1, part_name);

                    let dep_pos = mod_parts.iter().position(|nidx| *nidx == part1).unwrap();

                    mod_dep_matrix[part_pos * n_mod_parts + dep_pos] = true;
                    assert!(!mod_dep_matrix[dep_pos * n_mod_parts + part_pos]);
                } else {
                    let mod_dep_name = interner.insert_name(mod_dep_name);
                    let is_direct_dep = deps.contains(&mod_dep_name);
                    assert!(is_direct_dep);
                }
            }
        }
        info.deps.push(deps);
        info.mod_dep_matrix.push(mod_dep_matrix);
        info.module.push(mod_parts);

        let lib = scan_subfolder("cc", interner, &path.join("lib")).await;
        info.lib.push(lib);

        let test = scan_subfolder("cc", interner, &path.join("test")).await;
        info.test.push(test);

        let bin = scan_subfolder("cc", interner, &path.join("bin")).await;
        info.bin.push(bin);

        entry_idx += 1;
    }

    info
}

fn make_module_artifact_name(root_name: &str, part_name: &str, extension: &str) -> String {
    if root_name == part_name {
        format!("{}.{}", root_name, extension)
    } else {
        format!("{}-{}.{}", root_name, part_name, extension)
    }
}

fn plan_module_build(
    config: &Config,
    name_interner: &NameInterner,
    path_interner: &mut PathInterner,
    tasks: &mut Vec<Task>,
    info: &BuildInfo,
    entry_index: usize,
) {
    let mod_parts = info.module[entry_index].as_slice();
    let n_mod_parts = mod_parts.len();

    let entry_deps = info.deps[entry_index].as_slice();

    let mut ext_pcm_paths: Vec<PathIndex> = Vec::with_capacity(entry_deps.len());
    let mod_build_path = config.build_path.join("mod");
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
        let mod_dep_matrix_offset = n_mod_parts * entry_index;
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

    let root_pcm_path = config
        .build_path
        .join("mod")
        .join(format!("{}.pcm", entry_name));
    let root_pcm_path_index = path_interner.insert_path(&root_pcm_path);

    let mod_build_path = config.build_path.join("mod");
    let prebuilt_module_path_flag = format!("-fprebuilt-module-path={}", mod_build_path.display());

    let mod_parts = &info.lib[entry_index];

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

    let bins = &info.bin[entry_index];
    let build_path = config.build_path.join(entry_name).join(artifact_kind_name);
    let src_path = config.src_path.join(entry_name).join(artifact_kind_name);

    let mod_build_path = config.build_path.join("mod");
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
                            "Failed to run test:\n{}",
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

async fn run(executor: &'static Executor) {
    let config = parse_args();

    let _ = smol::fs::remove_dir_all(config.build_path).await;
    smol::fs::create_dir_all(config.build_path).await.unwrap();

    let mut name_interner = NameInterner::default();
    let info = scan(&config, &mut name_interner).await;
    let name_interner = name_interner;

    let mut path_interner = PathInterner::default();
    let tasks = plan(&config, &name_interner, &mut path_interner, &info);

    let build = Build::new(executor, tasks, path_interner.len());

    build.await;
}

fn main() {
    let executor = Box::leak(Box::new(Executor::new()));
    smol::future::block_on(executor.run(run(executor)));
}
