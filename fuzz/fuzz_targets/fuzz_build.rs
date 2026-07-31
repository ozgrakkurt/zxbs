#![no_main]

use libfuzzer_sys::arbitrary::unstructured::Unstructured;
use libfuzzer_sys::fuzz_target;

use interner::{Interner, Str};
use rand::seq::SliceRandom;
use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use zxbs::{build, command, interner};

use rand::SeedableRng as _;

static GLOBAL_PATHS: OnceLock<(Interner, Vec<Str>)> = OnceLock::new();

const N_PATHS: usize = 4096;

fn get_global_paths() -> &'static (Interner, Vec<Str>) {
    GLOBAL_PATHS.get_or_init(|| {
        let mut interner = Interner::new();
        let mut paths = Vec::with_capacity(N_PATHS);

        for i in 0..N_PATHS {
            paths.push(interner.insert(i.to_string().as_str()));
        }

        (interner, paths)
    })
}

fn make_task(
    finished: Arc<Mutex<HashSet<Str>>>,
    inputs: Vec<Str>,
    outputs: Vec<Str>,
) -> build::Task {
    build::Task {
        inputs: inputs.clone(),
        outputs: outputs.clone(),
        command: Some(command::Command::TestCmd(command::TestCmd {
            inputs,
            outputs,
            finished,
        })),
    }
}

fuzz_target!(|fuzz_input: &[u8]| {
    const MAX_N_TASKS: usize = 1024;

    let (interner, paths) = get_global_paths();
    let mut interner: Interner = interner.clone();

    let mut fuzz_input = Unstructured::new(fuzz_input);

    let mut tasks = Vec::with_capacity(MAX_N_TASKS);

    let mut path_idx = 0;

    let n_tasks = match fuzz_input.choose_index(MAX_N_TASKS) {
        Ok(n_tasks) => n_tasks,
        Err(_) => return,
    };

    let finished: Arc<Mutex<HashSet<Str>>> = Arc::new(Mutex::new(HashSet::new()));

    for _ in 0..n_tasks {
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
        tasks.push(make_task(finished.clone(), inputs, outputs));
    }
    assert!(path_idx <= N_PATHS);

    let rng_seed = match fuzz_input.choose_index(usize::try_from(u64::MAX).unwrap()) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut rng = rand::rngs::StdRng::seed_from_u64(u64::try_from(rng_seed).unwrap());

    tasks.shuffle(&mut rng);

    let build = build::Build::new(None, &mut interner, tasks);

    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(build);

    let finished = finished.lock().unwrap();
    for idx in paths.iter().take(path_idx) {
        assert!(finished.contains(idx));
    }
});
