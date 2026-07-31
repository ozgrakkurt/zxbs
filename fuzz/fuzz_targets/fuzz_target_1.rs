#![no_main]

use libfuzzer_sys::fuzz_target;

use std::sync::{Mutex, Arc}:
use rand::seq::SliceRandom;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use zxbs::{build, command, interner, tracer};

fuzz_target!(|data: &[u8]| {
    // fuzzed code goes here
});

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
        build::Task {
            inputs: vec![].leak(),
            outputs: vec![one].leak(),
            command: Some(command::Command::Dyn(Box::pin({
                let finished = finished.clone();
                async move {
                    let mut finished = finished.lock().unwrap();
                    finished.insert(one);
                }
            }))),
        },
        build::Task {
            inputs: vec![one].leak(),
            outputs: vec![two].leak(),
            command: Some(command::Command::Dyn(Box::pin({
                let finished = finished.clone();
                async move {
                    let mut finished = finished.lock().unwrap();
                    assert!(finished.contains(&one));
                    finished.insert(two);
                }
            }))),
        },
        build::Task {
            inputs: vec![one, two].leak(),
            outputs: vec![three].leak(),
            command: Some(command::Command::Dyn(Box::pin({
                let finished = finished.clone();
                async move {
                    let mut finished = finished.lock().unwrap();
                    assert!(finished.contains(&one));
                    assert!(finished.contains(&two));
                    finished.insert(three);
                }
            }))),
        },
        build::Task {
            inputs: vec![three].leak(),
            outputs: vec![four].leak(),
            command: Some(command::Command::Dyn(Box::pin({
                let finished = finished.clone();
                async move {
                    let mut finished = finished.lock().unwrap();
                    assert!(finished.contains(&three));
                    finished.insert(four);
                }
            }))),
        },
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

