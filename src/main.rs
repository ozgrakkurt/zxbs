#![allow(clippy::single_match)]
#![allow(clippy::wrong_self_convention)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::manual_range_contains)]

use smol::future::Boxed as BoxedFuture;
use smol::prelude::*;
use smol::stream::Boxed as BoxedStream;
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

    cflags.push(format!("-O{}", &cfg.opt_level).leak());

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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PathIndex(u32);

impl PathIndex {
    fn from_usize(v: usize) -> PathIndex {
        PathIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

#[derive(Clone, Copy, PartialEq)]
struct EntryIndex(u32);

impl EntryIndex {
    fn from_usize(v: usize) -> EntryIndex {
        EntryIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

#[derive(Clone, Copy, PartialEq)]
struct TaskIndex(u32);

impl TaskIndex {
    fn from_usize(v: usize) -> TaskIndex {
        TaskIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

struct PathInterner {
    map: HashMap<&'static str, PathIndex>,
    paths: Vec<&'static str>,
}

impl PathInterner {
    fn new() -> PathInterner {
        PathInterner {
            map: HashMap::new(),
            paths: Vec::new(),
        }
    }

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

    fn get_path(&mut self, idx: PathIndex) -> &'static Path {
        Path::new(self.paths[idx.to_usize()])
    }

    fn len(&self) -> usize {
        self.paths.len()
    }
}

type Command = BoxedFuture<()>;
type MakeCommandFn = Box<dyn Fn() -> Command + 'static + Send>;

struct Task {
    input_indices: &'static [PathIndex],
    output_indices: &'static [PathIndex],
    make_command: MakeCommandFn,
}

// type Job = BoxedFuture<TaskIndex>;

// #[must_use = "futures do nothing unless you `.await` or poll them"]
// struct Build {
//     scan: Option<Scan>,
//     path_interner: PathInterner,
//     is_built: Vec<bool>,
//     dependencies: Vec<Vec<PathIndex>>,
//     dependants: Vec<Vec<TaskIndex>>,
//     tasks: Vec<Task>,
//     running: Vec<Job>,
//     backlog: VecDeque<TaskIndex>,
//     concurrency: usize,
//     done: bool,
// }

// impl Build {
//     fn new(cfg: &'static Config) -> Self {
//         Self {
//             scan: Some(Scan::new(cfg)),
//             path_interner: PathInterner::new(),
//             is_built: Vec::new(),
//             dependencies: Vec::new(),
//             dependants: Vec::new(),
//             tasks: Vec::new(),
//             running: Vec::new(),
//             backlog: VecDeque::new(),
//             concurrency: num_cpus::get(),
//             done: false,
//         }
//     }

//     fn make_job(&self, task_idx: TaskIndex) -> Job {
//         let cmd = (self.tasks[task_idx.to_usize()].make_command)();
//         Box::pin(async move {
//             cmd.await;
//             task_idx
//         })
//     }

//     fn process_task_scanned(&mut self, task: Task) {
//         assert_eq!(self.tasks.len(), self.dependencies.len());

//         let task_idx = TaskIndex::from_usize(self.tasks.len());

//         self.is_built.resize(self.path_interner.len(), false);

//         let mut dependencies = Vec::with_capacity(task.input_indices.len());
//         for &path_idx in task.input_indices {
//             if !self.is_built[path_idx.to_usize()] {
//                 dependencies.push(path_idx);
//             }
//         }

//         self.dependants.resize(self.path_interner.len(), Vec::new());
//         for path_idx in task.input_indices {
//             self.dependants[path_idx.to_usize()].push(task_idx);
//         }

//         if dependencies.is_empty() {
//             self.backlog.push_back(task_idx);
//         }

//         self.tasks.push(task);
//         self.dependencies.push(dependencies);
//     }

//     fn process_task_finished(&mut self, task_idx: TaskIndex) {
//         let task = &self.tasks[task_idx.to_usize()];
//         for out_idx in task.output_indices {
//             assert!(!self.is_built[out_idx.to_usize()]);
//             self.is_built[out_idx.to_usize()] = true;

//             for &dependant_idx in self.dependants[out_idx.to_usize()].iter() {
//                 let deps = &mut self.dependencies[dependant_idx.to_usize()];
//                 let pos = deps.iter().position(|idx| idx.0 == out_idx.0).unwrap();
//                 deps.swap_remove(pos);

//                 if deps.is_empty() {
//                     self.backlog.push_back(dependant_idx);
//                 }
//             }
//         }
//     }
// }

// impl Future for Build {
//     type Output = ();

//     fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//         let fut = self.get_mut();

//         assert!(!fut.done);

//         if let Some(mut scan) = fut.scan.take() {
//             let mut tasks: Vec<Task> = Vec::new();
//             let mut done = false;
//             loop {
//                 match scan.poll_next(cx, &mut fut.path_interner) {
//                     Poll::Ready(Some(tasks_)) => {
//                         assert!(!tasks_.is_empty());
//                         tasks.extend(tasks_);
//                     }
//                     Poll::Ready(None) => {
//                         done = true;
//                         break;
//                     }
//                     Poll::Pending => {
//                         break;
//                     }
//                 }
//             }

//             for task in tasks {
//                 fut.process_task_scanned(task);
//             }

//             if !done {
//                 fut.scan = Some(scan);
//             }
//         }

//         let mut idx = 0;
//         while idx < fut.running.len() {
//             let job = &mut fut.running[idx];
//             match job.as_mut().poll(cx) {
//                 Poll::Ready(task_idx) => {
//                     fut.process_task_finished(task_idx);
//                     std::mem::drop(fut.running.swap_remove(idx));
//                 }
//                 Poll::Pending => {
//                     idx += 1;
//                 }
//             }
//         }

//         while fut.running.len() < fut.concurrency {
//             let task_idx = match fut.backlog.pop_front() {
//                 Some(task_idx) => task_idx,
//                 None => break,
//             };

//             let mut job = fut.make_job(task_idx);

//             match job.as_mut().poll(cx) {
//                 Poll::Ready(task_idx) => {
//                     fut.process_task_finished(task_idx);
//                 }
//                 Poll::Pending => {
//                     fut.running.push(job);
//                 }
//             }
//         }

//         if fut.scan.is_none() && fut.running.is_empty() && fut.backlog.is_empty() {
//             fut.done = true;
//             Poll::Ready(())
//         } else {
//             Poll::Pending
//         }
//     }
// }

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

async fn read_deps_file(path: &Path) -> Vec<String> {
    let contents = String::from_utf8(smol::fs::read(path).await.unwrap()).unwrap();
    contents.trim().split('\n').map(|s| s.to_owned()).collect()
}

#[derive(Default)]
struct BuildInfo {
    map: HashMap<PathIndex, EntryIndex>,
    path: Vec<PathIndex>,
    module: Vec<Vec<PathIndex>>,
    // matrix of mod_count * mod_count.
    // matrix[i * mod_count + j] is true if mod no i depends on mod no j.
    mod_dep_matrix: Vec<Vec<bool>>,
    lib: Vec<Vec<PathIndex>>,
    bin: Vec<Vec<PathIndex>>,
    test: Vec<Vec<PathIndex>>,
    deps: Vec<Vec<PathIndex>>,
}

impl BuildInfo {
    fn insert_entry(&mut self, path: PathIndex) -> EntryIndex {
        if let Some(eidx) = self.map.get(&path) {
            *eidx
        } else {
            let eidx = EntryIndex::from_usize(self.path.len());
            self.path.push(path);
            self.map.insert(path, eidx);
            eidx
        }
    }
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
    path_interner: &mut PathInterner,
    path: &Path,
) -> (Vec<String>, Vec<PathIndex>) {
    let mut dir = smol::fs::read_dir(path).await.unwrap();
    let mut paths = Vec::new();
    let mut names = Vec::new();
    while let Some(file) = dir.next().await {
        let file = file.unwrap();
        let name =
            parse_file_name(expected_extensions, file.file_name().to_str().unwrap()).to_owned();
        let file_path = file.path();
        let path_idx = path_interner.insert_path(&file_path);
        paths.push(path_idx);
        names.push(name);
    }

    (names, paths)
}

async fn scan(config: &Config, path_interner: &mut PathInterner) -> BuildInfo {
    let mut dir = smol::fs::read_dir(config.src_path).await.unwrap();

    let mut info = BuildInfo::default();

    while let Some(entry) = dir.next().await {
        let entry = entry.unwrap();
        let path = entry.path();
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_str().unwrap();
        check_name(entry_name);
        let path_idx = path_interner.insert_path(&path);

        let entry_idx = info.insert_entry(path_idx);

        let dep_names = read_deps_file(&path.join("deps")).await;
        let mut deps = Vec::with_capacity(dep_names.len());
        for dep_name in dep_names.iter() {
            let dep_path = config.src_path.join(dep_name);
            deps.push(path_interner.insert_path(&dep_path));
        }
        info.deps.push(deps);

        let (mod_part_names, module) =
            scan_subfolder(&["cppm"], path_interner, &path.join("mod")).await;

        let mut mod_dep_matrix = vec![false; module.len() * module.len()];
        for (self_idx, &mod_file_id) in module.iter().enumerate() {
            let mod_dep_names = scan_deps(config, path_interner.get_path(mod_file_id)).await;
            for mod_dep_name in mod_dep_names.iter() {
                if mod_dep_name.contains(':') {
                    let parts: Vec<_> = mod_dep_name.split(':').collect();
                    check_name(parts[1]);
                    assert_eq!(parts.len(), 2);
                    assert_eq!(parts[0], entry_name);
                    assert_ne!(parts[1], &mod_part_names[self_idx]);
                    let dep_idx = mod_part_names
                        .iter()
                        .position(|nm| nm.as_str() == parts[1])
                        .unwrap();
                    mod_dep_matrix[self_idx * module.len() + dep_idx] = true;
                    assert!(!mod_dep_matrix[dep_idx * module.len() + self_idx]);
                } else if !dep_names.contains(mod_dep_name) && mod_dep_name != entry_name {
                    panic!("invalid dependency");
                }
            }
        }
        info.mod_dep_matrix.push(mod_dep_matrix);
        info.module.push(module);

        let (_, lib) = scan_subfolder(&["s", "cc"], path_interner, &path.join("lib")).await;
        info.lib.push(lib);

        let (_, test) = scan_subfolder(&["cc"], path_interner, &path.join("test")).await;
        info.test.push(test);

        let (_, bin) = scan_subfolder(&["cc"], path_interner, &path.join("bin")).await;
        info.bin.push(bin);
    }

    info
}

fn plan(config: &Config, info: &BuildInfo) -> Build {
    
}

async fn run() {
    let config = parse_args();

    let mut path_interner = PathInterner::new();

    let info = scan(&config, &mut path_interner).await;

    let build = plan(&config, &info);
}

fn main() {
    smol::block_on(run());
}
