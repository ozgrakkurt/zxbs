#![no_main]

use libfuzzer_sys::arbitrary::unstructured::Unstructured;
use libfuzzer_sys::fuzz_target;

use rand::SeedableRng as _;
use rand::seq::SliceRandom;
use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use zxbs::interner::{Interner, Str};
use zxbs::thread_pool::ThreadPool;
use zxbs::{build, command};

const MAX_N_TASKS: usize = 1024;
const N_PATHS: usize = 4096;
// Every task gets an extra unique output so the dummy command can assert that
// each task ran exactly once, including tasks without real outputs.
const TOTAL_PATHS: usize = N_PATHS + MAX_N_TASKS;
const N_POOL_THREADS: usize = 4;

static GLOBAL_PATHS: OnceLock<(Interner, Vec<Str>)> = OnceLock::new();
static GLOBAL_THREAD_POOL: OnceLock<ThreadPool> = OnceLock::new();

fn get_global_paths() -> &'static (Interner, Vec<Str>) {
    GLOBAL_PATHS.get_or_init(|| {
        let mut interner = Interner::new();
        let mut paths = Vec::with_capacity(TOTAL_PATHS);

        for i in 0..TOTAL_PATHS {
            paths.push(interner.insert(i.to_string().as_str()));
        }

        (interner, paths)
    })
}

fn get_global_thread_pool() -> &'static ThreadPool {
    GLOBAL_THREAD_POOL.get_or_init(|| ThreadPool::new(N_POOL_THREADS))
}

fn make_task(
    entry_name: Str,
    part_name: Str,
    finished: Arc<Mutex<HashSet<Str>>>,
    inputs: Vec<Str>,
    outputs: Vec<Str>,
    sentinel: Str,
) -> build::Task {
    let mut command_outputs = outputs.clone();
    command_outputs.push(sentinel);

    let mut task_outputs = outputs;
    task_outputs.push(sentinel);

    build::Task {
        entry_name,
        part_name,
        entry_index: 0,
        inputs: inputs.clone(),
        outputs: task_outputs,
        command: Some(command::Command::TestCmd(command::TestCmd {
            inputs,
            outputs: command_outputs,
            finished,
        })),
    }
}

fuzz_target!(|fuzz_input: &[u8]| {
    let (interner, paths) = get_global_paths();
    let mut interner: Interner = interner.clone();
    let entry_name = interner.insert("fuzz_entry");
    let part_name = interner.insert("fuzz_part");

    let thread_pool = get_global_thread_pool();

    let mut fuzz_input = Unstructured::new(fuzz_input);

    let mut tasks = Vec::with_capacity(MAX_N_TASKS);

    let mut path_idx = 0;

    let n_tasks = match fuzz_input.choose_index(MAX_N_TASKS) {
        Ok(n_tasks) => n_tasks,
        Err(_) => return,
    };

    let finished: Arc<Mutex<HashSet<Str>>> = Arc::new(Mutex::new(HashSet::new()));

    for task_idx in 0..n_tasks {
        let n_paths = match fuzz_input.choose_index((N_PATHS - path_idx).min(10)) {
            Ok(np) => np,
            Err(_) => return,
        };

        let n_inputs = match fuzz_input.choose_index(n_paths) {
            Ok(ni) => ni,
            Err(_) => return,
        };
        let n_outputs = n_paths - n_inputs;

        let mut outputs = Vec::with_capacity(n_outputs);
        for i in 0..n_outputs {
            outputs.push(paths[path_idx + i]);
        }

        let mut inputs = Vec::with_capacity(n_inputs);
        for _ in 0..n_inputs {
            let in_idx = match fuzz_input.choose_index(path_idx) {
                Ok(in_idx) => in_idx,
                Err(_) => return,
            };
            inputs.push(paths[in_idx]);
        }

        path_idx += n_outputs;
        tasks.push(make_task(
            entry_name,
            part_name,
            finished.clone(),
            inputs,
            outputs,
            paths[N_PATHS + task_idx],
        ));
    }
    assert!(path_idx <= N_PATHS);

    let rng_seed = match fuzz_input.choose_index(usize::try_from(u64::MAX).unwrap()) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut rng = rand::rngs::StdRng::seed_from_u64(u64::try_from(rng_seed).unwrap());

    tasks.shuffle(&mut rng);

    let concurrency = match fuzz_input.choose_index(8) {
        Ok(v) => v + 1,
        Err(_) => return,
    };

    build::Build::new_with_concurrency(
        None,
        &mut interner,
        tasks,
        Instant::now(),
        thread_pool,
        concurrency,
    )
    .build();

    let finished = finished.lock().unwrap();
    for idx in paths.iter().take(path_idx) {
        assert!(finished.contains(idx));
    }
    for idx in paths[N_PATHS..N_PATHS + n_tasks].iter() {
        assert!(finished.contains(idx));
    }
});
