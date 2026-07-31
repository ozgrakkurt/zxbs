use std::collections::HashMap;
use zxbs::config::Config;

#[test]
fn test_empty() {
    Config::parse_args(HashMap::new());
}
