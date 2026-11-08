use zxbs::config::Config;
use zxbs::hash::HashMap;

#[test]
fn test_empty() {
    Config::parse_config(HashMap::default());
}

#[test]
fn test_simple() {
    let mut flags = HashMap::default();
    flags.insert("--build-dir".to_owned(), "/build123".to_owned());
    flags.insert("--src-dir".to_owned(), "/src123".to_owned());
    let cfg = Config::parse_config(flags);

    assert_eq!(cfg.build_path, "/build123/out");
    assert_eq!(cfg.src_path, "/src123");
}
