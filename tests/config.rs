use zxbs::config::Config;
use std::collections::HashMap;

#[test]
fn test_empty() {
    Config::parse_args(HashMap::new());    
}

#[test]
fn test_simple() {
    let mut args = HashMap::new();
    args.insert("--build-dir".to_owned(), "/build123".to_owned());
    args.insert("--src-dir".to_owned(), "/src123".to_owned());
    let cfg = Config::parse_args(args);

    assert_eq!(cfg.build_path, "/build123");
    assert_eq!(cfg.src_path, "/src123");
}
