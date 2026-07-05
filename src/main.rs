#![allow(clippy::single_match)]

use smol::prelude::*;
use std::task::{Poll, Context};
use std::future::Future;
use std::pin::Pin;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

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

    let cxx = args
        .get("--cxx")
        .map(|s| -> &'static str { s.leak() })
        .unwrap_or("clang++");
    let ar = args.get("--ar").map(|s| s.leak()).unwrap_or("llvm-ar");
    let clang_scan_deps = args
        .get("--clang-scan-deps")
        .map(|s| s.leak())
        .unwrap_or("clang-scan-deps");
    let opt_level = args.get("--opt-level").map(|s| s.leak()).unwrap_or("2");
    let dev_mode = get_bool_arg(&args, "dev-mode", true);
    let debug_info = get_bool_arg(&args, "debug-info", true);
    let fuzz = get_bool_arg(&args, "fuzz", false);
    let src_dir = args.get("--src-dir").map(|s| s.leak()).unwrap_or("src");
    let build_dir: &'static str = args.get("--build-dir").map(|s| s.leak()).unwrap_or("build");

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

struct PathIndex(u32);

impl PathIndex {
    fn from_usize(v: usize) -> PathIndex {
        PathIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

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
        match (self.map.get(path)) {
            Some(idx) => idx,
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

type JobFn = Box<dyn Fn() -> () + 'static>;

struct Task {
    input_indices: &'static [PathIndex],
    output_indices: &'static [PathIndex],
    do_job: Option<JobFn>,
}

type Job = Box<Future<Output = TaskIndex> + 'static>;

#[derive(Default)]
struct State {
    path_interner: PathInterner,
    dependencies: Vec<Vec<PathIndex>>,
    dependants: Vec<Vec<TaskIndex>>,
    tasks: Vec<Task>,
    ready_queue: VecDeque<TaskIndex>,
    scan_finished: bool,
}

impl State {
    fn pop_ready(&mut self) -> Option<Job> {
        if let Some(task_idx) = self.ready_queue.pop_front() {
            let do_job = self.tasks[task_id].do_job.take().unwrap();
            Some(Box::new(async move {
                do_job().await();
                task_idx
            })
        } else {
            None
        }
    }

    fn add_path(&mut self, path: &'static str) -> PathIndex {
        self.path_interner.insert_path(path)
    }

    fn add_task(
        &mut self,
        inputs: &[&'static str],
        outputs: &[&'static str],
        make_command: MakeCommandFn,
    ) -> TaskIndex {
        let idx = TaskIndex::from_usize(self.tasks.len());

        let input_indices = inputs
            .into_iter()
            .map(|path| self.path_interner.insert_path(path))
            .collect::<Vec<PathIndex>>();
        let output_indices = outputs
            .into_iter()
            .map(|path| self.path_interner.insert_path(path))
            .collect::<Vec<PathIndex>>();

        self.dependants.resize(self.path_interner.len(), Vec::new());
        self.dependencies[idx.to_usize()] = input_indices.clone();

        for input_idx in input_indices {
            self.dependants[input_idx.to_usize()].push(idx);
        }

        self.tasks[idx.to_usize()] = Task {
            input_indices: input_indices.leak(),
            output_indices: output_indices.leak(),
            make_command,
        };

        if self.dependencies[idx.to_usize()].is_empty() {
            self.ready_queue.push_back(idx);
        }

        idx
    }
}

#[derive(Default)]
struct Build {
    inner: RefCell<BuildInner>,
    concurrency: usize,
    running: futures::future::FuturesUnordered<Box<Command>>,
}

impl Build {
    async fn cycle(&self) -> bool {
        let fut: Box<dyn Future<Output=()> + 'static>  = {
            let inner = self.inner.borrow_mut().unwrap();
            if let Some(task_idx) = inner.ready_queue.pop_front() {
                inner.tasks[task_idx.to_usize()].make_command()
            } else {
                return false;
            }
        };

        if self.running.len() >= concurrency {
            self.running.next().await.unwrap();
        }
        self.running.push(fut);

        return true;
    }

    fn add_path(&self, path: &'static str) -> PathIndex {
        self.inner.borrow_mut().unwrap().add_path(path)
    }

    fn add_task(
        &self,
        inputs: &[&'static str],
        outputs: &[&'static str],
        make_command: MakeCommandFn,
    ) -> TaskIndex {
        self.inner
            .borrow_mut()
            .unwrap()
            .add_task(inputs, outputs, make_command)
    }
}

async fn build(recv: smol::channel::Receiver<super::TaskDescr>) {
    let concurrency = num_cpus::get();
    let running = futures::future::FuturesUnordered::new();

    while let Some(descr) = recv.next().await {
        
    }

    while running.len() > 0 {
        running.next().await.unwrap();
    }
}

struct PathIndex(u32);

impl PathIndex {
    fn from_usize(v: usize) -> PathIndex {
        PathIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

struct TaskIndex(u32);

impl TaskIndex {
    fn from_usize(v: usize) -> TaskIndex {
        TaskIndex(u32::try_from(v).unwrap())
    }

    fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

#[dervie(default)]
struct PathDedup {
    set: HashSet<&'static str>,
}

impl PathDedup {
    fn insert_path(&mut self, path: &str) -> &'static str {
        match self.set.get(path) {
            Some(out) => out,
            None => {
                let out = path.to_owned().leak();
                self.set.insert(out);
                out
            }
        }
    }
}

#[derive(Default)]
struct PathInterner {
    map: HashMap<&'static str, PathIndex>,
    paths: Vec<&'static str>,
}

impl PathInterner {
    fn insert_path(&mut self, path: &'static str) -> PathIndex {
        match (self.map.get(path)) {
            Some(idx) => idx,
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

struct Task {
    input_indices: &'static [PathIndex],
    output_indices: &'static [PathIndex],
    make_command: super::MakeCommandFn,
}

type Command = Box<Future<Output = ()> + 'static>;

#[derive(Default)]
struct State {
}

impl State {
    fn add_path(&mut self, path: &'static str) -> PathIndex {
        self.path_interner.insert_path(path)
    }

    fn add_task(
        &mut self,
        inputs: &[&'static str],
        outputs: &[&'static str],
        make_command: MakeCommandFn,
    ) -> TaskIndex {
        let idx = TaskIndex::from_usize(self.tasks.len());

        let input_indices = inputs
            .into_iter()
            .map(|path| self.path_interner.insert_path(path))
            .collect::<Vec<PathIndex>>();
        let output_indices = outputs
            .into_iter()
            .map(|path| self.path_interner.insert_path(path))
            .collect::<Vec<PathIndex>>();

        self.dependants.resize(self.path_interner.len(), Vec::new());
        self.dependencies[idx.to_usize()] = input_indices.clone();

        for input_idx in input_indices {
            self.dependants[input_idx.to_usize()].push(idx);
        }

        self.tasks[idx.to_usize()] = Task {
            input_indices: input_indices.leak(),
            output_indices: output_indices.leak(),
            make_command,
        };

        if self.dependencies[idx.to_usize()].is_empty() {
            self.ready_queue.push_back(idx);
        }

        idx
    }
}

#[derive(Default)]
struct Build {
    inner: RefCell<BuildInner>,
    concurrency: usize,
    running: futures::future::FuturesUnordered<Box<Command>>,
}

impl Build {
    async fn cycle(&self) -> bool {
        let fut: Box<dyn Future<Output=()> + 'static>  = {
            let inner = self.inner.borrow_mut().unwrap();
            if let Some(task_idx) = inner.ready_queue.pop_front() {
                inner.tasks[task_idx.to_usize()].make_command()
            } else {
                return false;
            }
        };

        if self.running.len() >= concurrency {
            self.running.next().await.unwrap();
        }
        self.running.push(fut);

        return true;
    }

    fn add_path(&self, path: &'static str) -> PathIndex {
        self.inner.borrow_mut().unwrap().add_path(path)
    }

    fn add_task(
        &self,
        inputs: &[&'static str],
        outputs: &[&'static str],
        make_command: MakeCommandFn,
    ) -> TaskIndex {
        self.inner
            .borrow_mut()
            .unwrap()
            .add_task(inputs, outputs, make_command)
    }
}

async fn build(recv: smol::channel::Receiver<super::TaskDescr>) {
    let concurrency = num_cpus::get();
    let running = futures::future::FuturesUnordered::new();

    while let Some(descr) = recv.next().await {
        
    }

    while running.len() > 0 {
        running.next().await.unwrap();
    }
}

struct ScanCpp {
    
}

impl Future for ScanCppm {
    
}

struct ScanCppm {
    
}

impl Future for ScanCppm {}

struct ScanEntry {
    
}

impl Stream for ScanEntry {
    
}

struct Scan {
    
}

impl Stream for Scan {
    type Item = TaskDescr;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        
    }
}

type Job = Box<dyn Future<Output=TaskIndex> + 'static + Send>;

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
}

impl Build {
    fn new(cfg: &Config) -> Self {
        Self {
            scan: Scan::new(cfg),
            path_internew: PathInterner::default(),
            dependencies: Vec::new(),
            dependants: Vec::new(),
            tasks: Vec::new(),
            running: Vec::new(),
            backlog: VecDeque::new(),
            concurrency: num_cpu::get(),
        }
    }

    fn make_job(&self, task_idx: TaskIndex) -> Job {
        let cmd = self.tasks[task_idx.to_usize()].make_command();
        Box::new(async move {
            cmd.await;
            task_idx
        })
    }

    fn process_task_descr(&mut self, descr: TaskDescr) {
        let task_idx = TaskIndex::from_usize(self.tasks.len());

        if dependencies.is_empty() {
            self.backlog.push_back(task_idx);
        }

        self.dependencies.push(dependencies);
        self.tasks.push(Task {
            
        });
    }

    fn process_task_finished(&mut self, task_idx: TaskIndex) {
        
    }
}

impl Future for Build {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(scan) = self.scan.get_mut() {
            match scan.poll_next(cx) {
                Poll::Ready(Some(task_descr)) => {
                    self.process_task_descr(task_descr);
                }
                Poll::Ready(None) => {
                    self.scan = None;
                }
                Poll::Pending => {}
            }
        }

        let mut idx = 0;
        while idx < self.running.len() {
            let job = &mut self.running[idx];
            match job.poll(cx) {
                Poll::Ready(task_idx) => {
                    self.process_task_finished(task_idx);
                    self.running.swap_remove(idx);
                }
                Poll::Pending => {
                    idx += 1;
                }
            }
        }

        while self.running.len() < self.concurrency {
            let task_idx = match self.backlog.pop_front() {
                Some(task_idx) => task_idx,
                None => break,
            };

            let mut job = self.make_job(task_idx);

            match job.poll(cx) {
                Poll::Ready(task_idx) => {
                    self.process_task_finished(task_idx);
                }
                Poll::Pending => {
                    self.running.push(job);
                }
            }
        }

        if self.scan.is_none() && self.running.is_empty() && self.backlog.is_empty() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

fn main() {
    let cfg = parse_args();

    let build: Box<dyn Future<Output=()> + 'static> = Box::new(Build::new(cfg));

    smol::LocalExecutor::new().run(build);
}
