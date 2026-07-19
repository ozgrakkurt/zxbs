#![allow(clippy::single_match)]
#![allow(clippy::wrong_self_convention)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::manual_range_contains)]

use smol::future::Boxed as BoxedFuture;
use smol::prelude::*;
use std::collections::{HashMap, VecDeque};
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

    fn len(&self) -> usize {
        self.names.len()
    }
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
    input_indices: &'static [PathIndex],
    output_indices: &'static [PathIndex],
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

        for (idx, task) in tasks.iter().enumerate() {
            let task_idx = TaskIndex::from_usize(idx);
            dependencies.push(task.input_indices.to_vec());
            for input_idx in task.input_indices.iter() {
                dependants[input_idx.to_usize()].push(task_idx);
            }
        }

        Build {
            executor,
            is_built: vec![false; tasks.len()],
            dependencies,
            dependants,
            tasks,
            running: Vec::new(),
            backlog: VecDeque::new(),
            concurrency: num_cpus::get(),
            done: false,
        }
    }

    fn process_task_finished(&mut self, task_idx: TaskIndex) {
        let task = &self.tasks[task_idx.to_usize()];
        for out_idx in task.output_indices {
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
    let output = smol::process::Command::new(config.clang_scan_deps)
        .arg("-format=p1689")
        .arg("--")
        .arg(config.cxx)
        .args(config.cflags)
        .arg(path)
        .output()
        .await
        .unwrap();
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
        for p in rule.get("provides").unwrap().as_array().unwrap().iter() {
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
    contents
        .trim()
        .split('\n')
        .map(|s| interner.insert_name(s))
        .collect()
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

fn parse_file_name<'name>(expected_extensions: &[&str], file_name: &'name str) -> &'name str {
    let parts: Vec<_> = file_name.split(".").collect();
    assert_eq!(parts.len(), 2);
    let name = parts[0];
    let extension = parts[1];

    check_name(name);

    assert!(expected_extensions.contains(&extension));

    name
}

async fn scan_subfolder(
    expected_extensions: &[&str],
    interner: &mut NameInterner,
    path: &Path,
) -> Vec<NameIndex> {
    let mut dir = smol::fs::read_dir(path).await.unwrap();
    let mut names = Vec::new();
    while let Some(file) = dir.next().await {
        let file = file.unwrap();
        let name =
            parse_file_name(expected_extensions, file.file_name().to_str().unwrap()).to_owned();
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

        let mod_parts = scan_subfolder(&["cppm"], interner, &path.join("mod")).await;
        let n_mod_parts = mod_parts.len();

        let mut mod_dep_matrix = vec![false; n_mod_parts * n_mod_parts];
        for (part_pos, &part_name) in mod_parts.iter().enumerate() {
            let mod_dep_names = scan_deps(config, &path.join(interner.get_name(part_name))).await;
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

        let lib = scan_subfolder(&["s", "cc"], interner, &path.join("lib")).await;
        info.lib.push(lib);

        let test = scan_subfolder(&["cc"], interner, &path.join("test")).await;
        info.test.push(test);

        let bin = scan_subfolder(&["cc"], interner, &path.join("bin")).await;
        info.bin.push(bin);

        entry_idx += 1;
    }

    info
}

// fn plan_module_build(
//     tasks: &mut Vec<Task>,
//     config: &Config,
//     entry_name: &str,
//     entry_deps: &[String],
//     module_parts: &[PathIndex],
//     mod_dep_matrix: &[bool],
// ) {
//     for part in module_parts {
//         let tidx = tasks.len();
//         let task_index = TaskIndex::from_usize(tidx);

//         precompile_cppm();
//         compile_pcm();

//         // precompile cppm to pcm
//         tasks.push(Task {
//             input_indices: [],
//             output_indices: [],
//             command: Box::pin(async move {
//                 // do the build job
//                 task_index
//             }),
//         });

//         // compile pcm to obj
//     }
// }

fn plan(
    config: &Config,
    name_interner: &NameInterner,
    path_interner: &mut PathInterner,
    info: &BuildInfo,
) -> Vec<Task> {
    // assert_eq!(info.path.len(), info.mod_dep_matrix.len());
    // assert_eq!(info.path.len(), info.module.len());
    // assert_eq!(info.path.len(), info.lib.len());
    // assert_eq!(info.path.len(), info.bin.len());
    // assert_eq!(info.path.len(), info.test.len());
    // assert_eq!(info.path.len(), info.deps.len());

    // let mut tasks = Vec::new();

    // for eidx in 0..info.path.len() {
    //     let entry_idx = EntryIndex::from_usize(eidx);
    //     plan_module_build(
    //         &tasks,
    //         config,
    //         &info.name[eidx],
    //         &info.deps[eidx],
    //         &info.module[eidx],
    //         &info.mod_dep_matrix[eidx],
    //     );
    // }

    // tasks
    Vec::new()
}

async fn run(executor: &'static Executor) {
    let config = parse_args();

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
