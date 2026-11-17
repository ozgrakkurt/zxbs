#![no_main]

use libfuzzer_sys::arbitrary::unstructured::Unstructured;
use libfuzzer_sys::fuzz_target;

use zxbs::command::CommandKind;
use zxbs::hash::HashMap;
use zxbs::interner::Interner;
use zxbs::tracer::{Entry, Tracer};

const KINDS: [CommandKind; 9] = [
    CommandKind::CompileModule,
    CommandKind::CompileToObject,
    CommandKind::ArchiveLib,
    CommandKind::LinkBinary,
    CommandKind::RunBinary,
    CommandKind::RunTest,
    CommandKind::CacheLoad,
    CommandKind::CacheStore,
    CommandKind::TestCmd,
];

fn count(fuzz_input: &mut Unstructured, max: usize) -> usize {
    fuzz_input.choose_index(max).unwrap_or(0)
}

fuzz_target!(|data: &[u8]| {
    let mut fuzz_input = Unstructured::new(data);
    let mut interner = Interner::new();

    let begin = fuzz_input.arbitrary::<u64>().unwrap_or(0);
    let mut tracer = Tracer::new(begin);

    let n_entries = count(&mut fuzz_input, 20);
    for _ in 0..n_entries {
        let entry_idx = count(&mut fuzz_input, 4);
        let part_idx = count(&mut fuzz_input, 4);
        let entry_name = interner.insert(&format!("e{}", entry_idx));
        let part_name = interner.insert(&format!("p{}", part_idx));

        tracer.record(Entry {
            start: fuzz_input.arbitrary::<u64>().unwrap_or(0),
            duration: fuzz_input.arbitrary::<u64>().unwrap_or(0),
            entry_name,
            part_name,
            kind: KINDS[count(&mut fuzz_input, KINDS.len())],
            entry_index: entry_idx,
            slot_index: count(&mut fuzz_input, 8),
        });
    }

    let mut timings: HashMap<String, (u64, u64)> = HashMap::default();
    let n_timings = count(&mut fuzz_input, 5);
    for idx in 0..n_timings {
        timings.insert(
            format!("timing{}", idx),
            (
                fuzz_input.arbitrary::<u64>().unwrap_or(0),
                fuzz_input.arbitrary::<u64>().unwrap_or(0),
            ),
        );
    }

    let json = tracer.print_json(&interner, &timings);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    let events = parsed.get("traceEvents").unwrap().as_array().unwrap();
    assert_eq!(events.len(), n_timings + n_entries);
    assert_eq!(
        parsed.get("beginningOfTime").unwrap().as_u64().unwrap(),
        begin
    );

    for event in events.iter() {
        assert_eq!(event.get("pid").unwrap().as_u64().unwrap(), 0);
        assert_eq!(event.get("ph").unwrap().as_str().unwrap(), "X");
        assert!(event.get("dur").unwrap().is_u64());
        assert!(event.get("ts").unwrap().is_u64());
        assert!(event.get("tid").unwrap().is_u64());
        assert!(event.get("name").unwrap().is_string());
        assert!(event.get("cat").unwrap().is_string());
        assert!(event.get("cname").unwrap().is_string());
    }
});
