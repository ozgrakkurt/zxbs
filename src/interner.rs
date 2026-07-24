use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Str(u32);

impl Str {
    pub fn from_usize(v: usize) -> Str {
        Str(u32::try_from(v).unwrap())
    }

    pub fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

pub struct Interner {
    map: HashMap<&'static str, Str>,
    list: Vec<&'static str>,
}

impl Interner {
    pub fn new() -> Interner {
        Interner {
            map: HashMap::new(),
            list: Vec::new(),
        }
    }

    pub fn insert(&mut self, item: &str) -> Str {
        match self.map.get(item) {
            Some(idx) => *idx,
            None => {
                let item: &'static str = item.to_owned().leak();
                let out = Str::from_usize(self.list.len());
                self.list.push(item);
                self.map.insert(item, out);
                out
            }
        }
    }

    pub fn get(&self, s: Str) -> &'static str {
        self.list[s.to_usize()]
    }
}
