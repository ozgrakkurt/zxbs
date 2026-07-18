#![allow(clippy::single_match)]
#![allow(clippy::wrong_self_convention)]
#![allow(clippy::ptr_arg)]

use smol::future::Boxed as BoxedFuture;
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

struct PathInterner {
    map: HashMap<&'static str, PathIndex>,
    paths: Vec<&'static str>,
}

impl PathInterner {
    fn new() -> PathInterner {
        return PathInterner {
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

#[must_use = "streams do nothing unless you poll them"]
enum ReadDir {
    Start(BoxedFuture<smol::io::Result<smol::fs::ReadDir>>),
    Running(BoxedStream<smol::io::Result<smol::fs::DirEntry>>),
    Done,
}

impl ReadDir {
    fn new(path: PathBuf) -> ReadDir {
        ReadDir::Start(Box::pin(smol::fs::read_dir(path)))
    }

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<smol::fs::DirEntry>> {
        loop {
            match std::mem::replace(self, ReadDir::Done) {
                ReadDir::Start(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Ready(res) => {
                        *self = ReadDir::Running(Box::pin(res.unwrap()));
                    }
                    Poll::Pending => {
                        *self = ReadDir::Start(fut);
                        return Poll::Pending;
                    }
                },
                ReadDir::Running(mut rd) => match rd.as_mut().poll_next(cx) {
                    Poll::Ready(Some(res)) => {
                        *self = ReadDir::Running(rd);
                        return Poll::Ready(Some(res.unwrap()));
                    }
                    Poll::Ready(None) => {
                        *self = ReadDir::Done;
                        return Poll::Ready(None);
                    }
                    Poll::Pending => {
                        *self = ReadDir::Running(rd);
                        return Poll::Pending;
                    }
                },
                ReadDir::Done => unreachable!(),
            }
        }
    }
}

enum ScanModState {
    WaitEntry,
    ScanDeps(PathIndex, BoxedFuture<Vec<String>>),
    Done,
}

// Scan mod folder under a source entry.
// Iterates every file in the mod folder, each have to be a cppm.
// Generates two tasks, one for precompiling into pcm, another for compiling the pcm into object file.
// First task depends on the local module dependencies of this module and library dependencies of the
// source entry.
// Second task depends on the first task's pcm output.
#[must_use = "streams do nothing unless you poll them"]
struct ScanMod {
    name: String,
    path: PathBuf,
    read_dir: ReadDir,
}

impl ScanMod {
    fn new(path: PathBuf) -> ScanMod {
        let name = path.file_name();
        ScanMod {
            read_dir: ReadDir::new(path),
        }
    }

    fn poll_next(
        &mut self,
        cx: &mut Context<'_>,
        path_interner: &mut PathInterner,
        config: &'static Config,
        entry: &smol::fs::DirEntry,
        object_indices: &mut Vec<PathIndex>,
    ) -> Poll<Option<Vec<Task>>> {
        loop {
            match std::mem::replace(&mut self.state, ScanModState::Done) {
                ScanModState::Done => unreachable!(),
                ScanModState::WaitEntry => match self.read_dir.poll_next(cx) {
                    Poll::Ready(Some(entry)) => {
                        self.state = ScanModState::ScanDeps(Box::pin(scan_deps(config, )));
                    }
                    Poll::Ready(None) => {
                        self.state = ScanModState::Done;
                        return Poll::Ready(None);
                    }
                    Poll::Pending => {
                        self.state = ScanModState::WaitEntry;
                        return Poll::Pending;
                    }
                },
                ScanModState::ScanDeps(df) => match df.poll(cx) {
                    Poll::Ready(deps) => {
                        self.state = ScanModState::WaitEntry;
                        return Poll::Ready(Some(task));
                    }
                    Poll::Pending => {
                        self.state = ScanModState::ScanDeps(df);
                        return Poll::Pending;
                    }
                },
            }
        }
    }
}

#[must_use = "streams do nothing unless you poll them"]
enum ScanEntryState {
    ReadDepsJson(BoxedFuture<DepsJson>),
    ScanMod(ScanMod),
    ScanLib(ReadDir),
    ScanTest(ReadDir),
    ScanBin(ReadDir),
    Done,
}

struct ScanEntry {
    config: &'static Config,
    deps: Vec<PathIndex>,
    test_deps: Vec<PathIndex>,
    deps: Option<&'static [PathIndex]>,
    object_indices: Vec<PathIndex>,
    entry: smol::fs::DirEntry,
    state: ScanEntryState,
}

impl ScanEntry {
    fn new(entry: smol::fs::DirEntry, config: &'static Config) -> ScanEntry {
        let rdj = Box::pin(read_deps_json(entry.path().join("deps.json")));

        ScanEntry {
            entry,
            config,
            state: ScanEntryState::ReadDepsJson(rdj),
            deps: None,
            object_indices: Vec::new(),
        }
    }

    fn poll_next(
        &mut self,
        cx: &mut Context<'_>,
        path_interner: &mut PathInterner,
    ) -> Poll<Option<Vec<Task>>> {
        loop {
            match std::mem::replace(&mut self.state, ScanEntryState::Done) {
                ScanEntryState::ReadDepsJson(mut rdj) => match rdj.as_mut().poll(cx) {
                    Poll::Ready(deps_json) => {
                        deps_json
                        self.deps = Some(deps_json);
                        self.state =
                            ScanEntryState::ScanMod(ReadDir::new(self.entry.path().join("mod")));
                    }
                    Poll::Pending => {
                        self.state = ScanEntryState::ReadDepsJson(rdj);
                        return Poll::Pending;
                    }
                },
                ScanEntryState::ScanMod(mut rd) => match rd.poll_next(cx) {
                    Poll::Ready(Some(entry)) => {
                        self.state = ScanEntryState::ScanMod(rd);
                        return Poll::Ready(Some(todo!()));
                    }
                    Poll::Ready(None) => {
                        self.state =
                            ScanEntryState::ScanLib(ReadDir::new(self.entry.path().join("lib")));
                    }
                    Poll::Pending => {
                        self.state = ScanEntryState::ScanMod(rd);
                        return Poll::Pending;
                    }
                },
                ScanEntryState::ScanLib(mut rd) => match rd.poll_next(cx) {
                    Poll::Ready(Some(entry)) => {
                        self.state = ScanEntryState::ScanLib(rd);
                        return Poll::Ready(Some(todo!()));
                    }
                    Poll::Ready(None) => {
                        self.state =
                            ScanEntryState::ScanTest(ReadDir::new(self.entry.path().join("test")));
                        let object_indices = self.object_indices.clone().leak();
                        let object_paths: &'static [&'static str] = object_indices
                            .iter()
                            .map(|i| path_interner.get_path(*i))
                            .collect::<Vec<&'static str>>()
                            .leak();
                        let lib_name = self.entry.file_name();
                        let out_path = self
                            .config
                            .build_path
                            .join(&lib_name)
                            .join(format!("lib{}.a", lib_name.display()));
                        let out_idx = path_interner.insert_path(out_path.to_str().unwrap());
                        let out_path = path_interner.get_path(out_idx);
                        let ar = self.config.ar;
                        return Poll::Ready(Some(vec![Task {
                            input_indices: object_indices,
                            output_indices: vec![out_idx].leak(),
                            make_command: Box::new(move || {
                                Box::pin(async move {
                                    assert!(smol::process::Command::new(ar)
                                        .arg("crs")
                                        .arg(out_path)
                                        .args(object_paths)
                                        .output()
                                        .await
                                        .unwrap()
                                        .status.success());
                                })
                            }),
                        }]));
                    }
                    Poll::Pending => {
                        self.state = ScanEntryState::ScanLib(rd);
                        return Poll::Pending;
                    }
                },
                ScanEntryState::ScanTest(mut rd) => match rd.poll_next(cx) {
                    Poll::Ready(Some(entry)) => {
                        self.state = ScanEntryState::ScanTest(rd);
                        return Poll::Ready(Some(todo!()));
                    }
                    Poll::Ready(None) => {
                        self.state =
                            ScanEntryState::ScanBin(ReadDir::new(self.entry.path().join("bin")));
                    }
                    Poll::Pending => {
                        self.state = ScanEntryState::ScanTest(rd);
                        return Poll::Pending;
                    }
                },
                ScanEntryState::ScanBin(mut rd) => match rd.poll_next(cx) {
                    Poll::Ready(Some(entry)) => {
                        self.state = ScanEntryState::ScanBin(rd);
                        return Poll::Ready(Some(todo!()));
                    }
                    Poll::Ready(None) => {
                        return Poll::Ready(None);
                    }
                    Poll::Pending => {
                        self.state = ScanEntryState::ScanBin(rd);
                        return Poll::Pending;
                    }
                },
                ScanEntryState::Done => unreachable!(),
            }
        }
    }
}

#[must_use = "streams do nothing unless you poll them"]
struct Scan {
    read_dir: Option<ReadDir>,
    running: Vec<ScanEntry>,
    to_return: Vec<Task>,
    concurrency: usize,
    config: &'static Config,
    // deps context
    // these fields are used for book-keeping about
    // dependency chain between source entries.
    // 
    // We read the deps.json when we find a source entry and
    // the reading of deps.json is recorded in the read_deps vector.
    //
    // Then information is recorded in the deps/test-deps/bin-deps vectors.
    // And dependency chain is collected when we are issuing an entry read for the
    // source entry. We also check for circular dependency on self when doing this
    // collection.
    read_deps_json: Vec<BoxedFuture<(PathBuf, DepsJson)>>,
    deps: Vec<PathIndex>,
    test_deps: Vec<PathIndex>,
    bin_deps: Vec<PathIndex>,
    // reversed dependency relation for issuing entry scans when dependency waiting is resolved.
    wait_rev_deps: Vec<Vec<PathIndex>>,
    // for waiting for all dependencies to be resolved. We only can issue the entry scan when
    // all dependencies are resolved since we will need the whole dependency chain to
    // construct the build commands and entryScan is supposed to be able to create the build commands.
    wait_deps: Vec<Vec<PathIndex>>,
}

impl Scan {
    fn new(cfg: &'static Config) -> Scan {
        Scan {
            read_dir: Some(ReadDir::new(cfg.src_path.to_owned())),
            running: Vec::new(),
            to_return: Vec::new(),
            concurrency: num_cpus::get(),
            config: cfg,
            read_deps_json: Vec::new(),
            deps: Vec::new(),
            test_deps: Vec::new(),
            bin_deps: Vec::new(),
            wait_rev_deps: Vec::new(),
            wait_deps: Vec::new(),
        }
    }

    fn poll_next(
        &mut self,
        cx: &mut Context<'_>,
        path_interner: &mut PathInterner,
    ) -> Poll<Option<Vec<Task>>> {
        if let Some(mut rd) = self.read_dir.take() {
            let mut done = false;
            loop {
                match rd.poll_next(cx) {
                    Poll::Ready(Some(dir_entry)) => {
                        let path = dir_entry.path().join("deps.json");
                        let cfg = self.config;
                        let mut rdj = Box::pin(async move {
                            (read_deps_json(cfg, path))
                        });

                        match rdj.as_mut().poll(cx) {
                           Poll::Ready(_) => {
                               unreachable!();
                           }
                           Poll::Pending => {
                               self.read_deps_json.push_back(rdj);
                           }  
                        }                        
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
            if !done {
                self.read_dir = Some(rd);
            } else {
                assert!(self.wait_deps.is_empty());
            }
        }

        let mut idx = 0;
        while idx < self.running.len() {
            let scan_entry = &mut self.running[idx];
            let mut done = false;
            loop {
                match scan_entry.poll_next(cx, path_interner) {
                    Poll::Ready(Some(tasks)) => {
                        assert!(!tasks.is_empty());
                        self.to_return.extend(tasks);
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

            if done {
                std::mem::drop(self.running.swap_remove(idx));
            } else {
                idx += 1;
            }
        }

        while self.running.len() < self.concurrency {
            let dir_entry = match self.backlog.pop_front() {
                Some(dir_entry) => dir_entry,
                None => break,
            };

            let mut scan_entry = ScanEntry::new(dir_entry, self.config);

            loop {
                match scan_entry.poll_next(cx, path_interner) {
                    Poll::Ready(Some(tasks)) => {
                        assert!(!tasks.is_empty());
                        self.to_return.extend(tasks);
                    }
                    Poll::Ready(None) => {
                        break;
                    }
                    Poll::Pending => {
                        self.running.push(scan_entry);
                        break;
                    }
                }
            }
        }

        if !self.to_return.is_empty() {
            return Poll::Ready(Some(std::mem::take(&mut self.to_return)));
        }

        if self.read_dir.is_none() && self.backlog.is_empty() && self.running.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
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
    fn new(cfg: &'static Config) -> Self {
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
                    Poll::Ready(Some(tasks_)) => {
                        assert!(!tasks_.is_empty());
                        tasks.extend(tasks_);
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

async fn scan_deps(config: &'static Config, path: &'static str) -> Vec<String> {
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

struct Object {
    id: PathIdx,
    deps: Vec<PathIdx>,
}

struct Entry {
    mod: Vec<Object>,
    lib: Vec<Object>,
    bin: Vec<Object>,
    test: Vec<Object>,
    deps: Vec<PathIdx>,
    test_deps: Vec<PathIdx>,
    bin_deps: Vec<PathIdx>,
}

struct BuildInfo {
    entries: Vec<Entry>,
}

async fn scan_src(cfg: &Config, path_interner: &mut PathInterner) -> BuildInfo {
    let dir = smol::fs::read_dir(cfg.src_path).await.unwrap();

    while let Some(res) = dir.next().await {
        let src_entry = res.unwrap();
        e.path();
        let dj = read_deps_json(cfg).await;

        let mod_dir = smol::fs::read_dir(e.path().join("mod")).await.unwrap();

        while let Some(res) = mod_dir.next().await {
            let mod_entry = res.unwrap();
            let deps = scan_deps(cfg, mod_entry).await;
            // check deps are self name or sub of self name;
        }

        while let Some(res) = mod_dir.next().await {
            
        }
    }
}

// Scan -> collect BuildInfo
// Plan -> create build tasks using the build information
// Build -> run the build tasks

async fn read_deps(path: &Path) -> Vec<String> {
    let contents = smol::fs::read(path).await.unwrap();
    contents.trim().split('\n').map(|s| s.to_owned()).collect()
}

#[derive(Default)]
struct BuildInfo {
    id: Vec<PathIndex>,
    mod: Vec<Vec<PathIndex>>,
    // matrix of mod_count * mod_count.
    // matrix[i * mod_count + j] is true if mod no i depends on mod no j.
    mod_dep_matrix: Vec<Vec<bool>>,
    lib: Vec<Vec<PathIndex>>,
    bin: Vec<Vec<PathIndex>>,
    test: Vec<Vec<PathIndex>>,
    // same idea as mod_dep_matrix
    dep_matrix: Vec<bool>,
}

fn check_name(name: &str) {
    for byte in name.as_bytes() {
        let is_char = byte >= b'a' && byte <= b'z';
        let is_underscore = byte == b'_';
        assert!(is_char || is_underscore);
    }
}

fn parse_file_name(expected_extension: &str, file_name: &str) -> &str {
    let parts: Vec<_> = file_name.split(".").collect();
    assert_eq!(parts.len(), 2);
    let name = parts[0];
    let extension = parts[1];

    assert!(!name.contains('-'));
    assert!(!name.contains('.'));
    assert!(!name.contains(':'));

    check_name(name);

    assert!(extension == expected_extension);

    name
}

fn check_file_name(expected_extension: &str, file_name: &str) {
    parse_file_name(expected_extension, file_name);
}

async fn scan_subfolder(path_interner: &mut PathInterner, path: &Path) -> Vec<PathIndex> {
    
}

async fn scan(config: &Config, path_interner: &mut PathInterner) -> BuildInfo {
    let dir = smol::fs::read_dir(config.src_path).await.unwrap();

    let mut info = BuildInfo::default();

    while let Some(entry) = dir.next().await {
        let entry = entry.unwrap();
        let path = entry.path();
        let id = path_interner.insert_path(path);
        info.id.push(id);

        let deps = read_deps(path.join("deps")).await;
        let mut dep_ids = Vec::with_capacity(deps.len());
        for d in deps.iter() {
            let dep_path = config.src_path.join(d);
        }

        let mod_path = path.join("mod");
        let mod_dir = smol::fs::read_dir(mod_path).await.unwrap();
        let mut mod = Vec::new();
        while let Some(mod_file) = dir.next().await {
            let mod_file = mod_file.unwrap();
            check_file_name(mod_file.file_name());
            let mod_file_path = mod_file.path();
            let mod_file_id = path_interner.insert_path(mod_file_path);
            mod.push(mod_file_id);
        }
        info.mod.push(mod);

        let mut mod_dep_matrix = vec![mod.len() * mod.len(); false];
        for &mod_file_id in mod.iter() {
            // clang_scan_deps
            // assert all deps are not self and are local or coming from a direct dependency.
            // do direct dependency check by name.
            // insert i depends on j
            // assert j doesn't depend on i
        }

        let lib_path = path.join("lib");
        let lib_dir = smol::fs::read_dir(lib_path).await.unwrap();
        let mut lib = Vec::new();
        while let Some(lib_file) = dir.next().await {
            let lib_file = lib_file.unwrap();
            check_file_name(lib_file.file_name());
            let lib_file_path = lib_file.path();
            let lib_file_id = path_interner.insert_path(lib_file_path);
            lib.push(lib_file_id);
        }
        info.lib.push(lib);
    }

    info
}

fn main() {
    let cfg = Box::leak(Box::new(parse_args()));

    let build: BoxedFuture<()> = Box::pin(Build::new(cfg));

    smol::block_on(build);
}
