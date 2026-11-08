use crate::hash::HashMap;
use crate::interner::{Interner, Str};

#[derive(Clone)]
pub struct Entry {
    pub start: u64,
    pub duration: u64,
    pub entry_name: Str,
    pub part_name: Str,
    pub kind: Str,
    pub entry_index: usize,
    pub slot_index: usize,
}

pub struct Tracer {
    data: Vec<Entry>,
    begin_timestamp: u64,
}

// These are defined somewhere in chrome://tracing format documentation/source code somewhere.
// Unfortunately I can't remember where it is at the time of writing this comment.
//
// Yes, just using 'gray' 'orange' etc. doesn't work.
const COLORS: [&str; 5] = [
    "thread_state_iowait",
    "thread_state_running",
    "thread_state_runnable",
    "thread_state_sleeping",
    "thread_state_unknown",
];

impl Tracer {
    pub fn new(begin_timestamp: u64) -> Tracer {
        Tracer {
            data: Vec::new(),
            begin_timestamp,
        }
    }

    pub fn record(&mut self, entry: Entry) {
        self.data.push(entry);
    }

    fn make_trace_event(
        &self,
        thread_id: usize,
        start: u64,
        duration: u64,
        name: String,
        category: String,
        color: String,
    ) -> serde_json::Value {
        let mut obj = serde_json::Map::<String, serde_json::Value>::new();

        obj.insert(
            "pid".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(0)),
        );
        obj.insert(
            "tid".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(thread_id)),
        );
        obj.insert(
            "ts".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(start)),
        );
        obj.insert(
            "ph".to_owned(),
            serde_json::Value::String(String::from("X")),
        );
        obj.insert(
            "dur".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(duration)),
        );
        obj.insert("name".to_owned(), serde_json::Value::String(name));
        obj.insert("cat".to_owned(), serde_json::Value::String(category));
        obj.insert("cname".to_owned(), serde_json::Value::String(color));

        serde_json::Value::Object(obj)
    }

    pub fn print_json(
        &self,
        interner: &Interner,
        scan_timing: (u64, u64),
        plan_timing: (u64, u64),
        build_timing: (u64, u64),
    ) -> String {
        let mut events: Vec<serde_json::Value> = Vec::with_capacity(self.data.len());

        events.push(self.make_trace_event(
            0,
            scan_timing.0,
            scan_timing.1,
            "Scan".to_owned(),
            "Scan".to_owned(),
            "generic_work".to_owned(),
        ));
        events.push(self.make_trace_event(
            0,
            plan_timing.0,
            plan_timing.1,
            "Plan".to_owned(),
            "Plan".to_owned(),
            "generic_work".to_owned(),
        ));
        events.push(self.make_trace_event(
            0,
            build_timing.0,
            build_timing.1,
            "Build".to_owned(),
            "Build".to_owned(),
            "generic_work".to_owned(),
        ));

        let mut color_mapping = HashMap::<Str, usize>::default();
        let mut color_slot: usize = 0;
        for entry in self.data.iter() {
            let color: String = match color_mapping.get(&entry.kind) {
                Some(v) => COLORS[*v].to_owned(),
                None => {
                    color_mapping.insert(entry.kind, color_slot);
                    let out = COLORS[color_slot].to_owned();
                    color_slot = (color_slot + 1) % COLORS.len();
                    out
                }
            };
            events.push(self.make_trace_event(
                entry.slot_index + 1,
                entry.start,
                entry.duration,
                format!(
                    "{}:{}",
                    interner.get(entry.entry_name),
                    interner.get(entry.part_name)
                ),
                interner.get(entry.kind).to_owned(),
                color,
            ));
        }

        let mut out = serde_json::Map::<String, serde_json::Value>::new();
        out.insert("traceEvents".to_owned(), serde_json::Value::Array(events));
        out.insert(
            "beginningOfTime".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(self.begin_timestamp)),
        );

        serde_json::to_string(&out).unwrap()
    }
}
