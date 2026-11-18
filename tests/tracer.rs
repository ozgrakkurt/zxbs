use zxbs::command::CommandKind;
use zxbs::hash::HashMap;
use zxbs::interner::Interner;
use zxbs::tracer::{Entry, Tracer};

// The chrome tracing color scheme indexes each command kind gets.
const KIND_COLORS: [(CommandKind, &str); 9] = [
    (CommandKind::CompileModule, "thread_state_running"),
    (CommandKind::CompileToObject, "thread_state_runnable"),
    (CommandKind::ArchiveLib, "thread_state_sleeping"),
    (CommandKind::LinkBinary, "thread_state_unknown"),
    (CommandKind::RunBinary, "memory_dump"),
    (CommandKind::RunTest, "generic_work"),
    (CommandKind::CacheLoad, "good"),
    (CommandKind::CacheStore, "bad"),
    (CommandKind::TestCmd, "terrible"),
];

fn trace_entry(interner: &mut Interner, kind: CommandKind, slot_index: usize) -> Entry {
    Entry {
        start: 100,
        duration: 200,
        entry_name: interner.insert("myapp"),
        part_name: interner.insert("main"),
        kind,
        entry_index: 0,
        slot_index,
    }
}

// The recorded entries and the top level timings must all end up as valid
// chrome tracing "X" (complete) events with the right thread ids, names and
// colors.
#[test]
fn test_tracer_print_json() {
    let mut interner = Interner::new();

    let mut tracer = Tracer::new(1234);

    // One recorded entry per command kind so every color mapping is checked.
    for (slot_index, &(kind, _)) in KIND_COLORS.iter().enumerate() {
        tracer.record(trace_entry(&mut interner, kind, slot_index));
    }

    let mut top_level_timings: HashMap<String, (u64, u64)> = HashMap::default();
    top_level_timings.insert("scan".to_owned(), (7, 8));
    top_level_timings.insert("plan".to_owned(), (9, 10));

    let json = tracer.print_json(&interner, &top_level_timings);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(value["beginningOfTime"], 1234);

    let events = value["traceEvents"].as_array().unwrap();
    assert_eq!(events.len(), 2 + KIND_COLORS.len());

    // Top level timings run on the main thread (tid 0) with the first color
    // of the scheme.
    for (name, &(ts, dur)) in top_level_timings.iter() {
        let event = events
            .iter()
            .find(|event| event["name"] == name.as_str())
            .unwrap();

        assert_eq!(event["pid"], 0, "{}", name);
        assert_eq!(event["tid"], 0, "{}", name);
        assert_eq!(event["ts"], ts, "{}", name);
        assert_eq!(event["dur"], dur, "{}", name);
        assert_eq!(event["ph"], "X", "{}", name);
        assert_eq!(event["cname"], "thread_state_iowait", "{}", name);
    }

    // Recorded entries are named `entry:part`, run on the thread of their
    // build slot (offset by one to make room for the main thread) and are
    // colored by their command kind.
    for (slot_index, &(kind, color)) in KIND_COLORS.iter().enumerate() {
        let event = events
            .iter()
            .find(|event| event["tid"] == (slot_index + 1))
            .unwrap();

        assert_eq!(event["name"], "myapp:main");
        assert_eq!(event["ts"], 100);
        assert_eq!(event["dur"], 200);
        assert_eq!(event["ph"], "X");
        assert_eq!(event["cat"], format!("{:?}", kind));
        assert_eq!(event["cname"], color);
    }
}

// An empty tracer still produces valid tracing json.
#[test]
fn test_tracer_empty() {
    let interner = Interner::new();
    let tracer = Tracer::new(42);

    let top_level_timings: HashMap<String, (u64, u64)> = HashMap::default();

    let json = tracer.print_json(&interner, &top_level_timings);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(value["beginningOfTime"], 42);
    assert_eq!(value["traceEvents"].as_array().unwrap().len(), 0);
}
