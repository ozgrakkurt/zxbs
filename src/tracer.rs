use std::collections::HashMap;

use crate::interner::{Interner, Str};

#[derive(Clone, Copy)]
pub struct Entry {
    pub timing_ms: u64,
    pub output: Str,
    pub kind: Str,
}

pub struct Tracer {
    enabled: bool,
    data: Vec<Entry>,
}

impl Tracer {
    pub fn new(enabled: bool) -> Tracer {
        Tracer {
            enabled,
            data: Vec::new(),
        }
    }

    pub fn record(&mut self, entry: Entry) {
        if self.enabled {
            self.data.push(entry);
        }
    }

    pub fn print_by_timing(&mut self, interner: &Interner) -> String {
        if !self.enabled {
            assert!(self.data.is_empty());
            return String::new();
        }

        if self.data.is_empty() {
            return String::new();
        }

        self.data.sort_unstable_by_key(|l| l.timing_ms);

        let mut out = String::with_capacity(1024);

        for entry in self.data.iter().rev().take(30) {
            out.push_str(interner.get(entry.output));
            out.push_str(":\t");
            out.push_str(entry.timing_ms.to_string().as_str());
            out.push_str("ms\n");
        }

        out
    }

    pub fn print_by_kind(&mut self, interner: &Interner) -> String {
        if !self.enabled {
            assert!(self.data.is_empty());
            return String::new();
        }

        if self.data.is_empty() {
            return String::new();
        }

        let mut by_kind: HashMap<Str, u64> = HashMap::new();

        for entry in self.data.iter() {
            *by_kind.entry(entry.kind).or_insert(0) += entry.timing_ms;
        }

        let mut by_kind = by_kind.into_iter().collect::<Vec<(Str, u64)>>();

        by_kind.sort_unstable_by_key(|l| l.1);

        let mut out = String::with_capacity(1024);

        for &(kind, timing_ms) in by_kind.iter().rev() {
            out.push_str(interner.get(kind));
            out.push_str(": ");
            out.push_str(timing_ms.to_string().as_str());
            out.push_str("ms\n");
        }

        out
    }
}
