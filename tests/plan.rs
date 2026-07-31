use std::collections::HashMap;

use zxbs::{config, interner, plan, scan};

#[test]
fn test_empty() {
    let cfg = config::Config::parse_args(HashMap::new());

    let mut interner = interner::Interner::new();

    let info = scan::BuildInfo::default();

    let tasks = plan::plan(&cfg, &mut interner, &info);

    assert!(tasks.is_empty());
}

#[test]
fn test_simple() {
    let cfg = config::Config::parse_args(HashMap::new());

    let mut interner = interner::Interner::new();

    let mut info = scan::BuildInfo::default();

    let baselib = interner.insert("base");
    let iolib = interner.insert("io");
    let myapp = interner.insert("myapp");

    info.map.insert(baselib, 0);
    info.name.push(baselib);
    info.module.push(vec![
        interner.insert("base"),
        interner.insert("vec"),
        interner.insert("start"),
        interner.insert("arith"),
        interner.insert("alloc"),
    ]);
    #[rustfmt::skip]
    info.mod_dep_matrix.push(vec![
        false, true, true, true, true,
        false, false, false, true, true,
        false, false, false, false, false,
        false, false, true, false, false,
        false, false, true, true, false,
    ]);
    info.lib
        .push(vec![interner.insert("memcmp"), interner.insert("memcpy")]);
    info.bin.push(vec![interner.insert("memcpy_bench")]);
    info.test.push(vec![
        interner.insert("memcmp"),
        interner.insert("vec"),
        interner.insert("arith"),
    ]);
    info.deps.push(vec![]);

    info.map.insert(iolib, 1);
    info.name.push(iolib);
    info.module.push(vec![
        interner.insert("io"),
        interner.insert("executor"),
        interner.insert("fs"),
        interner.insert("net"),
    ]);
    #[rustfmt::skip]
    info.mod_dep_matrix.push(vec![
        false, true, true, true,
        false, false, false, false,
        false, true, false, false,
        false, true, false, false,
    ]);
    info.lib.push(vec![]);
    info.bin.push(vec![interner.insert("direct_io_example")]);
    info.test.push(vec![
        interner.insert("executor_ordering"),
        interner.insert("fs"),
        interner.insert("net"),
    ]);
    info.deps.push(vec![baselib]);

    info.map.insert(myapp, 2);
    info.name.push(myapp);
    info.module.push(vec![]);
    info.mod_dep_matrix.push(vec![]);
    info.lib.push(vec![]);
    info.bin.push(vec![myapp]);
    info.test.push(vec![]);
    info.deps.push(vec![baselib, iolib]);

    let tasks = plan::plan(&cfg, &mut interner, &info);

}
