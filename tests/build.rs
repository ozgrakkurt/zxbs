use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rand::SeedableRng as _;
use rand::seq::SliceRandom;
use zxbs::build::{self, Task};
use zxbs::command::{self, Command};
use zxbs::interner::{Interner, Str};
use zxbs::thread_pool::ThreadPool;

fn make_task(
    interner: &mut Interner,
    finished: Arc<Mutex<HashSet<Str>>>,
    inputs: Vec<Str>,
    outputs: Vec<Str>,
) -> Task {
    Task {
        entry_name: interner.insert("dummy_entry"),
        part_name: interner.insert("dummy_part"),
        inputs: inputs.clone(),
        outputs: outputs.clone(),
        command: Some(Command::TestCmd(command::TestCmd {
            inputs,
            outputs,
            finished,
        })),
        entry_index: 69,
    }
}

#[test]
fn empty_run() {
    let thread_pool = ThreadPool::new(2);
    let mut interner = Interner::new();
    build::Build::new(
        None,
        &mut interner,
        Vec::new(),
        Instant::now(),
        &thread_pool,
    )
    .build();
}

#[test]
fn simple_run() {
    let thread_pool = ThreadPool::new(2);
    let mut interner = Interner::new();

    let one = interner.insert("1");
    let two = interner.insert("2");
    let three = interner.insert("3");
    let four = interner.insert("4");

    let finished = Arc::new(Mutex::new(HashSet::new()));

    let mut tasks = vec![
        make_task(&mut interner, finished.clone(), vec![], vec![one]),
        make_task(&mut interner, finished.clone(), vec![one], vec![two]),
        make_task(&mut interner, finished.clone(), vec![one, two], vec![three]),
        make_task(&mut interner, finished.clone(), vec![three], vec![four]),
    ];

    // Shuffle the tasks deterministically so a failure of this test is
    // reproducible. The build system has to come up with a correct schedule
    // no matter what order the tasks are given in.
    let mut rng = rand::rngs::StdRng::seed_from_u64(0);
    tasks.shuffle(&mut rng);

    build::Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();

    let finished = finished.lock().unwrap();
    assert!(finished.contains(&one));
    assert!(finished.contains(&two));
    assert!(finished.contains(&three));
    assert!(finished.contains(&four));
}
