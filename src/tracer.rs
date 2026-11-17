use crate::command::CommandKind;
use crate::hash::HashMap;
use crate::interner::{Interner, Str};

#[derive(Clone)]
pub struct Entry {
    pub start: u64,
    pub duration: u64,
    pub entry_name: Str,
    pub part_name: Str,
    pub kind: CommandKind,
    pub entry_index: usize,
    pub slot_index: usize,
}

pub struct Tracer {
    data: Vec<Entry>,
    begin_timestamp: u64,
}

// These are supposed to be defined somewhere in chrome://tracing format documentation/source code somewhere.
//
// Yes, just using 'gray' 'orange' etc. doesn't work.
//
// Taken from:
// https://chromium.googlesource.com/external/trace-viewer/+/bf55211014397cf0ebcd9e7090de1c4f84fc3ac0/tracing/tracing/ui/base/color_scheme.html
const COLORS: [&str; 25] = [
    "thread_state_iowait",
    "thread_state_running",
    "thread_state_runnable",
    "thread_state_sleeping",
    "thread_state_unknown",
    "memory_dump",
    "generic_work",
    "good",
    "bad",
    "terrible",
    "black",
    "rail_response",
    "rail_animate",
    "rail_idle",
    "rail_load",
    "used_memory_column",
    "older_used_memory_column",
    "tracing_memory_column",
    "cq_build_running",
    "cq_build_passed",
    "cq_build_failed",
    "cq_build_abandoned",
    "cq_build_attempt_running",
    "cq_build_attempt_passed",
    "cq_build_attempt_failed",
];

fn command_kind_to_color(kind: CommandKind) -> &'static str {
    let idx = match kind {
        CommandKind::CompileModule => 1,
        CommandKind::CompileToObject => 2,
        CommandKind::ArchiveLib => 3,
        CommandKind::LinkBinary => 4,
        CommandKind::RunBinary => 5,
        CommandKind::RunTest => 6,
        CommandKind::CacheLoad => 7,
        CommandKind::CacheStore => 8,
        CommandKind::TestCmd => 9,
    };
    COLORS[idx]
}

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
        top_level_timings: &HashMap<String, (u64, u64)>,
    ) -> String {
        let mut events: Vec<serde_json::Value> = Vec::with_capacity(self.data.len());

        for (name, timing) in top_level_timings.iter() {
            events.push(self.make_trace_event(
                0,
                timing.0,
                timing.1,
                name.clone(),
                name.clone(),
                COLORS[0].to_owned(),
            ));
        }

        for entry in self.data.iter() {
            let color = command_kind_to_color(entry.kind).to_owned();
            events.push(self.make_trace_event(
                entry.slot_index + 1,
                entry.start,
                entry.duration,
                format!(
                    "{}:{}",
                    interner.get(entry.entry_name),
                    interner.get(entry.part_name)
                ),
                format!("{:?}", entry.kind),
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
