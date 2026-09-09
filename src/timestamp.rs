use std::time::{SystemTime, UNIX_EPOCH};

pub fn timestamp() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    u64::try_from(now.as_micros()).unwrap()
}
