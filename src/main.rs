#![allow(clippy::single_match)]

use smol::future::Boxed as BoxedFuture;
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

    return Config {
        cxx: cxx.to_owned().leak(),
        ar: ar.to_owned().leak(),
        clang_scan_deps: clang_scan_deps.to_owned().leak(),
        build_path,
        src_path,
        cflags,
        ldflags,
    };
}

#[derive(Clone, Copy, PartialEq)]
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
struct PathInterner {
    map: HashMap<&'static str, PathIndex>,
    paths: Vec<&'static str>,
}

impl PathInterner {
    fn insert_path(&mut self, path: &'static str) -> PathIndex {
        match self.map.get(path) {
            Some(idx) => *idx,
            None => {
                let idx = PathIndex::from_usize(self.paths.len());
                self.paths.push(path);
                self.map.insert(path, idx);
                idx
            }
        }
    }

    fn get_path(&mut self, idx: PathIndex) -> &'static str {
        self.paths[idx.to_usize()]
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

// type ScanFile = Box<dyn Future<Output = TaskDescr> + 'static>;

// struct ScanEntry {
//     read_dir: Option<smol::fs::ReadDir>,
//     running: Vec<ScanFile>,
//     concurrency: usize,
// }

// impl ScanEntry {
//     fn new(entry: smol::fs::DirEntry) -> ScanEntry {

//     }
// }

// impl Stream for ScanEntry {
//     type Item = TaskDescr;

//     fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {

//     }
// }

// enum ReadDir {
//     Start(BoxedFuture<smol::fs::ReadDir>),
//     Running(smol::fs::ReadDir),
//     Done,
// }

// impl ReadDir {
//     fn new(path: &Path) -> ReadDir {
//         ReadDir::Start(smol::fs::read_dir(path).boxed())
//     }

//     fn poll_next(self: &mut self, cx: &mut Context<'_>) -> Poll<Option<smol::fs::DirEntry>> {
//         loop {
//             match std::mem::swap(self, ) {

//             }
//         }
//     }
// }

// enum Scan {
//     Running {
//         read_dir: ScanReadDir,
//         running: Vec<ScanEntry>,
//         ret_queue: VecDeque<TaskDescr>,
//         backlog: VecDeque<smol::fs::DirEntry>,
//         concurrency: usize,
//     },
//     Done,
// }

// struct Scan {

// }

// impl Scan {
//     fn new(cfg: Config) -> Scan {
//         let read_dir = smol::fs::read_dir(cfg.src_path);
//         Scan::ReadDir()
//     }

//     fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>, path_interner: &mut PathInterner) -> Poll<Option<Task>> {
//         if let Some(rd) = self.read_dir.get_mut() {
//             while true {
//                 match rd.poll_next(cx) {
//                     Poll::Ready(Some(dir_entry)) => {
//                         self.backlog.push_back(dir_entry);
//                     }
//                     Poll::Ready(None) => {
//                         self.read_dir = None;
//                         break;
//                     }
//                     Poll::Pending => {
//                         break;
//                     }
//                 }
//             }
//         }

//         let mut idx = 0;
//         while idx < self.running.len() {
//             let mut scan_entry = &mut self.running[idx];
//             let mut done = false;
//             while true {
//                 match scan_entry.poll_next(cx) {
//                     Poll::Ready(Some(task_descr)) => {
//                         self.ret_queue.push_back(task_descr);
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

//             if done {
//                 self.running.swap_remove(idx);
//             }
//         }

//         while self.running.len() < self.concurrency {
//             let dir_entry = match self.backlog.pop_front() {
//                 Some(dir_entry) => dir_entry,
//                 None => break,
//             };

//             let mut scan_entry = ScanEntry::new(dir_entry);

//             while true {
//                 match scan_entry.poll_next(cx) {
//                     Poll::Ready(Some(task_descr)) => {
//                         self.ret_queue.push_back(task_descr);
//                     }
//                     Poll::Ready(None) => {
//                         break;
//                     }
//                     Poll::Pending => {
//                         self.running.push(scan_entry);
//                         break;
//                     }
//                 }
//             }
//         }

//         if let Some(task_descr) = self.ret_queue.pop_front() {
//             return Poll::Ready(Some(task_descr));
//         }

//         if self.read_dir.is_none() && self.backlog.is_empty() && self.running.is_empty() {
//             Poll::Ready(None)
//         } else {
//             Poll::Pending
//         }
//     }
// }

struct Scan {}

impl Scan {
    fn new(_cfg: &Config) -> Scan {
        Scan {}
    }

    fn poll_next(
        &mut self,
        _cx: &mut Context<'_>,
        _path_interner: &mut PathInterner,
    ) -> Poll<Option<Task>> {
        todo!()
    }
}

type Job = BoxedFuture<TaskIndex>;

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct Build {
    scan: Option<Scan>,
    path_interner: PathInterner,
    is_built: Vec<bool>,
    dependencies: Vec<Vec<PathIndex>>,
    dependants: Vec<Vec<TaskIndex>>,
    tasks: Vec<Task>,
    running: Vec<Job>,
    backlog: VecDeque<TaskIndex>,
    concurrency: usize,
    done: bool,
}

impl Build {
    fn new(cfg: &Config) -> Self {
        Self {
            scan: Some(Scan::new(cfg)),
            path_interner: PathInterner::default(),
            is_built: Vec::new(),
            dependencies: Vec::new(),
            dependants: Vec::new(),
            tasks: Vec::new(),
            running: Vec::new(),
            backlog: VecDeque::new(),
            concurrency: num_cpus::get(),
            done: false,
        }
    }

    fn make_job(&self, task_idx: TaskIndex) -> Job {
        let cmd = (self.tasks[task_idx.to_usize()].make_command)();
        Box::pin(async move {
            cmd.await;
            task_idx
        })
    }

    fn process_task_scanned(&mut self, task: Task) {
        assert_eq!(self.tasks.len(), self.dependencies.len());

        let task_idx = TaskIndex::from_usize(self.tasks.len());

        self.is_built.resize(self.path_interner.len(), false);

        let mut dependencies = Vec::with_capacity(task.input_indices.len());
        for &path_idx in task.input_indices {
            if !self.is_built[path_idx.to_usize()] {
                dependencies.push(path_idx);
            }
        }

        self.dependants.resize(self.path_interner.len(), Vec::new());
        for path_idx in task.input_indices {
            self.dependants[path_idx.to_usize()].push(task_idx);
        }

        if dependencies.is_empty() {
            self.backlog.push_back(task_idx);
        }

        self.tasks.push(task);
        self.dependencies.push(dependencies);
    }

    fn process_task_finished(&mut self, task_idx: TaskIndex) {
        let task = &self.tasks[task_idx.to_usize()];
        for out_idx in task.output_indices {
            assert!(!self.is_built[out_idx.to_usize()]);
            self.is_built[out_idx.to_usize()] = true;

            for &dependant_idx in self.dependants[out_idx.to_usize()].iter() {
                let deps = &mut self.dependencies[dependant_idx.to_usize()];
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

        if let Some(mut scan) = fut.scan.take() {
            let mut tasks: Vec<Task> = Vec::new();
            let mut done = false;
            loop {
                match scan.poll_next(cx, &mut fut.path_interner) {
                    Poll::Ready(Some(task)) => {
                        tasks.push(task);
                    }
                    Poll::Ready(None) => {
                        done = true;
                        break;
                    }
                    Poll::Pending => {
                        break;
                    }
                }
            }

            for task in tasks {
                fut.process_task_scanned(task);
            }

            if !done {
                fut.scan = Some(scan);
            }
        }

        let mut idx = 0;
        while idx < fut.running.len() {
            let job = &mut fut.running[idx];
            match job.as_mut().poll(cx) {
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

            let mut job = fut.make_job(task_idx);

            match job.as_mut().poll(cx) {
                Poll::Ready(task_idx) => {
                    fut.process_task_finished(task_idx);
                }
                Poll::Pending => {
                    fut.running.push(job);
                }
            }
        }

        if fut.scan.is_none() && fut.running.is_empty() && fut.backlog.is_empty() {
            fut.done = true;
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

fn main() {
    let cfg = parse_args();

    let build: BoxedFuture<()> = Box::pin(Build::new(&cfg));

    smol::block_on(build);
}
