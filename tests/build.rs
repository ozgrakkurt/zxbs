use interner::Str;
use rand::seq::SliceRandom;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use zxbs::{build, command, interner, tracer};

fn make_task(
    finished: Arc<Mutex<HashSet<Str>>>,
    inputs: Vec<Str>,
    outputs: Vec<Str>,
) -> build::Task {
    let inputs = inputs.leak();
    let outputs = outputs.leak();
    build::Task {
        inputs,
        outputs,
        command: Some(command::Command::TestCmd(command::TestCmd {
            inputs,
            outputs,
            finished,
        })),
    }
}

#[tokio::test]
async fn empty_run() {
    let mut tracer = tracer::Tracer::new(false);
    let mut interner = interner::Interner::new();
    let build = build::Build::new(&mut tracer, &mut interner, Vec::new());
    build.await;
}

#[tokio::test]
async fn simple_run() {
    let mut interner = interner::Interner::new();

    let one = interner.insert("1");
    let two = interner.insert("2");
    let three = interner.insert("3");
    let four = interner.insert("4");

    let finished = Arc::new(Mutex::new(HashSet::new()));

    let mut tasks = vec![
        make_task(finished.clone(), vec![], vec![one]),
        make_task(finished.clone(), vec![one], vec![two]),
        make_task(finished.clone(), vec![one, two], vec![three]),
        make_task(finished.clone(), vec![three], vec![four]),
    ];

    let mut rng = rand::rng();
    tasks.shuffle(&mut rng);

    let mut tracer = tracer::Tracer::new(false);
    let build = build::Build::new(&mut tracer, &mut interner, tasks);
    build.await;

    let finished = finished.lock().unwrap();
    assert!(finished.contains(&one));
    assert!(finished.contains(&two));
    assert!(finished.contains(&three));
    assert!(finished.contains(&four));
}
