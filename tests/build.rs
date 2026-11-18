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

// A build with zero tasks must run the scheduler loop to completion without
// tripping any of the scheduler's internal asserts ("couldn't find a starting
// task", "tasks remaining" etc). `Build::build` returns nothing and encodes
// all of its invariants as panics, so completing without a panic is the
// assertion.
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

#[test]
#[should_panic(expected = "Couldn't find a starting task")]
fn cyclic_tasks_are_rejected() {
    let thread_pool = ThreadPool::new(2);
    let mut interner = Interner::new();
    let finished = Arc::new(Mutex::new(HashSet::new()));

    let a = interner.insert("a");
    let b = interner.insert("b");

    let tasks = vec![
        make_task(&mut interner, finished.clone(), vec![b], vec![a]),
        make_task(&mut interner, finished.clone(), vec![a], vec![b]),
    ];

    build::Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();
}

#[test]
#[should_panic(expected = "Couldn't find a starting task")]
fn self_dependency_is_rejected() {
    let thread_pool = ThreadPool::new(2);
    let mut interner = Interner::new();
    let finished = Arc::new(Mutex::new(HashSet::new()));

    let a = interner.insert("a");
    let task = make_task(&mut interner, finished, vec![a], vec![a]);

    build::Build::new(
        None,
        &mut interner,
        vec![task],
        Instant::now(),
        &thread_pool,
    )
    .build();
}

#[test]
#[should_panic(expected = "was built for the second time")]
fn duplicate_outputs_are_rejected() {
    let thread_pool = ThreadPool::new(2);
    let mut interner = Interner::new();

    let a = interner.insert("a");

    // Use a command that doesn't inspect the task outputs itself so the
    // duplicate output is detected by the build scheduler.
    let make_task = |interner: &mut Interner| Task {
        entry_name: interner.insert("entry"),
        part_name: interner.insert("part"),
        inputs: Vec::new(),
        outputs: vec![a],
        command: Some(Command::RunBinary(command::RunBinary {
            path: "true",
            args: &[],
        })),
        entry_index: 0,
    };

    let tasks = vec![make_task(&mut interner), make_task(&mut interner)];

    build::Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();
}

#[test]
#[should_panic]
fn missing_producer_is_rejected() {
    let thread_pool = ThreadPool::new(2);
    let mut interner = Interner::new();
    let finished = Arc::new(Mutex::new(HashSet::new()));

    let a = interner.insert("a");
    let missing = interner.insert("missing");
    let b = interner.insert("b");

    let tasks = vec![
        make_task(&mut interner, finished.clone(), vec![], vec![a]),
        make_task(&mut interner, finished.clone(), vec![missing], vec![b]),
    ];

    build::Build::new(None, &mut interner, tasks, Instant::now(), &thread_pool).build();
}

#[test]
fn single_threaded_build_runs_every_task() {
    let thread_pool = ThreadPool::new(2);
    let mut interner = Interner::new();
    let finished = Arc::new(Mutex::new(HashSet::new()));

    let one = interner.insert("1");
    let two = interner.insert("2");
    let three = interner.insert("3");

    let tasks = vec![
        make_task(&mut interner, finished.clone(), vec![], vec![one]),
        make_task(&mut interner, finished.clone(), vec![one], vec![two]),
        make_task(&mut interner, finished.clone(), vec![two], vec![three]),
    ];

    build::Build::new_with_concurrency(None, &mut interner, tasks, Instant::now(), &thread_pool, 1)
        .build();

    let finished = finished.lock().unwrap();
    assert!(finished.contains(&one));
    assert!(finished.contains(&two));
    assert!(finished.contains(&three));
}
