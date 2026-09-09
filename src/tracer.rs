use crate::interner::{Interner, Str};

#[derive(Clone)]
pub struct Entry {
    pub timestamp: u64,
    pub outputs: Vec<Str>,
    pub kind: Str,
    pub entry_index: usize,
}

pub struct Tracer {
    data: Vec<Entry>,
    begin_timestamp: u64,
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
        timestamp: u64,
        name: String,
    ) -> serde_json::Value {
        let mut obj = serde_json::Map::<String, serde_json::Value>::new();

        obj.insert(
            "pid".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(69)),
        );
        obj.insert(
            "tid".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(thread_id)),
        );
        obj.insert(
            "ts".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(timestamp)),
        );
        obj.insert(
            "ph".to_owned(),
            serde_json::Value::String(String::from("X")),
        );
        obj.insert(
            "dur".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(
                timestamp.checked_sub(self.begin_timestamp).unwrap(),
            )),
        );
        obj.insert("name".to_owned(), serde_json::Value::String(name));

        serde_json::Value::Object(obj)
    }

    pub fn print_json(
        &self,
        interner: &Interner,
        scan_timestamp: u64,
        plan_timestamp: u64,
        build_timestamp: u64,
    ) -> String {
        let mut events: Vec<serde_json::Value> = Vec::with_capacity(self.data.len());

        events.push(self.make_trace_event(0, scan_timestamp, "Scan".to_owned()));
        events.push(self.make_trace_event(0, plan_timestamp, "Plan".to_owned()));
        events.push(self.make_trace_event(0, build_timestamp, "Build".to_owned()));

        for entry in self.data.iter() {
            let name: String = entry
                .outputs
                .iter()
                .map(|&s| interner.get(s))
                .collect::<Vec<_>>()
                .join("-");
            events.push(self.make_trace_event(entry.entry_index + 1, entry.timestamp, name));
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
