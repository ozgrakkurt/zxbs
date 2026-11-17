#![no_main]
#![allow(clippy::needless_range_loop)]

use libfuzzer_sys::arbitrary::unstructured::Unstructured;
use libfuzzer_sys::fuzz_target;

use std::path::Path;
use std::sync::OnceLock;
use zxbs::config::{Config, Subcommand};
use zxbs::hash::HashMap;
use zxbs::interner::{Interner, Str};
use zxbs::plan::collect_lib_path_list;
use zxbs::scan::{
    BuildInfo, try_check_cyclic_dependencies_between_entries,
    try_check_cyclic_dependencies_between_module_parts,
};

fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| Config {
        cxx: "",
        ar: "",
        clang_scan_deps: "",
        clang_format: "",
        cxxflags: &[],
        ldflags: &[],
        build_path: Path::new("/build"),
        src_path: Path::new("/src"),
        cache_path: Path::new("/cache"),
        time_trace: false,
        fuzz_unlimited: false,
    })
}

fn coin(fuzz_input: &mut Unstructured) -> bool {
    fuzz_input.choose_index(2).unwrap_or(0) == 1
}

fn count(fuzz_input: &mut Unstructured, max: usize) -> usize {
    fuzz_input.choose_index(max).unwrap_or(0)
}

fn make_info(
    names: &[Str],
    deps: Vec<Vec<Str>>,
    bin: Vec<Vec<Str>>,
    test: Vec<Vec<Str>>,
) -> BuildInfo {
    let n = names.len();
    let map: HashMap<Str, u32> = names
        .iter()
        .enumerate()
        .map(|(idx, &name)| (name, u32::try_from(idx).unwrap()))
        .collect();

    BuildInfo {
        map,
        name: names.to_vec(),
        module: vec![Vec::new(); n],
        mod_dep_matrix: vec![Vec::new(); n],
        lib: vec![Vec::new(); n],
        bin,
        test,
        deps,
        basic_hash: HashMap::default(),
    }
}

fn has_cycle(adj: &[Vec<usize>]) -> bool {
    fn visit(adj: &[Vec<usize>], state: &mut [u8], node: usize) -> bool {
        if state[node] == 1 {
            return true;
        }
        if state[node] == 2 {
            return false;
        }
        state[node] = 1;
        for &dep in adj[node].iter() {
            if visit(adj, state, dep) {
                return true;
            }
        }
        state[node] = 2;
        false
    }

    let mut state = vec![0u8; adj.len()];
    for node in 0..adj.len() {
        if state[node] == 0 && visit(adj, &mut state, node) {
            return true;
        }
    }
    false
}

fn lib_path(name: &str) -> String {
    format!("/build/entry/{}/lib/lib{}.a", name, name)
}

fuzz_target!(|data: &[u8]| {
    let mut fuzz_input = Unstructured::new(data);
    let mut interner = Interner::new();

    // Random directed graph, including cyclic graphs and self loops.
    let n = count(&mut fuzz_input, 32) + 1;
    let names_str: Vec<String> = (0..n).map(|i| format!("e{}", i)).collect();
    let names: Vec<Str> = names_str.iter().map(|name| interner.insert(name)).collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for from in 0..n {
        for to in 0..n {
            if coin(&mut fuzz_input) {
                adj[from].push(to);
            }
        }
    }

    let deps: Vec<Vec<Str>> = adj
        .iter()
        .map(|deps| deps.iter().map(|&idx| names[idx]).collect())
        .collect();
    let info = make_info(
        &names,
        deps.clone(),
        vec![Vec::new(); n],
        vec![Vec::new(); n],
    );

    // Both cycle checkers must agree with a simple reference DFS.
    let expected_cycle = has_cycle(&adj);
    let entry_result =
        try_check_cyclic_dependencies_between_entries(&interner, &names, &info.map, &deps);
    assert_eq!(entry_result.is_err(), expected_cycle);

    let matrix: Vec<bool> = (0..n * n)
        .map(|idx| adj[idx / n].contains(&(idx % n)))
        .collect();
    let module_result =
        try_check_cyclic_dependencies_between_module_parts(&interner, names[0], &names, &matrix);
    assert_eq!(module_result.is_err(), expected_cycle);

    // Link order: generate a DAG where edges only point to higher indices.
    let dag_n = count(&mut fuzz_input, 32) + 1;
    let dag_names_str: Vec<String> = (0..dag_n).map(|i| format!("e{}", i)).collect();
    let dag_names: Vec<Str> = dag_names_str
        .iter()
        .map(|name| interner.insert(name))
        .collect();

    let mut dag_adj: Vec<Vec<usize>> = vec![Vec::new(); dag_n];
    for from in 0..dag_n {
        for to in (from + 1)..dag_n {
            if coin(&mut fuzz_input) {
                dag_adj[from].push(to);
            }
        }
    }

    let dag_deps: Vec<Vec<Str>> = dag_adj
        .iter()
        .map(|deps| deps.iter().map(|&idx| dag_names[idx]).collect())
        .collect();
    let dag_info = make_info(
        &dag_names,
        dag_deps,
        vec![Vec::new(); dag_n],
        vec![Vec::new(); dag_n],
    );

    let mut reachable = vec![false; dag_n];
    let mut stack = vec![0usize];
    reachable[0] = true;
    while let Some(node) = stack.pop() {
        for &dep in dag_adj[node].iter() {
            if !reachable[dep] {
                reachable[dep] = true;
                stack.push(dep);
            }
        }
    }
    let reachable_count = reachable.iter().filter(|&&v| v).count();

    let path_list = collect_lib_path_list(config(), &mut interner, &dag_info, 0);
    assert_eq!(path_list.len(), reachable_count);

    let mut path_to_idx = std::collections::HashMap::new();
    for (idx, name) in dag_names_str.iter().enumerate() {
        path_to_idx.insert(lib_path(name), idx);
    }

    let mut positions = vec![usize::MAX; dag_n];
    for (pos, path) in path_list.iter().enumerate() {
        let idx = *path_to_idx.get(*path).unwrap();
        assert!(reachable[idx]);
        assert_eq!(positions[idx], usize::MAX, "duplicate library path");
        positions[idx] = pos;
    }

    for from in 0..dag_n {
        if !reachable[from] {
            continue;
        }
        for &dep in dag_adj[from].iter() {
            assert!(positions[from] < positions[dep], "bad link order");
        }
    }

    // The order must be deterministic.
    let path_list2 = collect_lib_path_list(config(), &mut interner, &dag_info, 0);
    assert_eq!(path_list, path_list2);

    // Subcommand validation: compare against a direct membership check.
    let sub_n = count(&mut fuzz_input, 4) + 1;
    let sub_names_str: Vec<String> = (0..sub_n).map(|i| format!("e{}", i)).collect();
    let sub_names: Vec<Str> = sub_names_str
        .iter()
        .map(|name| interner.insert(name))
        .collect();

    let mut bin: Vec<Vec<Str>> = vec![Vec::new(); sub_n];
    let mut test: Vec<Vec<Str>> = vec![Vec::new(); sub_n];
    for entry in 0..sub_n {
        for part in 0..5 {
            if coin(&mut fuzz_input) {
                let name = format!("p{}", part);
                bin[entry].push(interner.insert(&name));
            }
            if coin(&mut fuzz_input) {
                let name = format!("p{}", part);
                test[entry].push(interner.insert(&name));
            }
        }
    }

    let sub_info = make_info(&sub_names, vec![Vec::new(); sub_n], bin, test);

    let kind = count(&mut fuzz_input, 6);
    let entry = if coin(&mut fuzz_input) {
        sub_names_str[count(&mut fuzz_input, sub_n)].clone()
    } else {
        "missing_entry".to_owned()
    };
    let part = if coin(&mut fuzz_input) {
        format!("p{}", count(&mut fuzz_input, 5))
    } else {
        "missing_part".to_owned()
    };

    let entry_idx = sub_names_str.iter().position(|name| *name == entry);
    let part_idx = interner.insert(&part);
    let expected = match kind {
        0 => true, // TestAll
        1 => true, // Build
        // `fmt` and `clean` are handled in main.rs before scanning, so the
        // scan-time subcommand check rejects them.
        2 => false, // Fmt
        3 => entry_idx.is_some_and(|idx| sub_info.bin[idx].contains(&part_idx)),
        4 => entry_idx.is_some_and(|idx| sub_info.test[idx].contains(&part_idx)),
        5 => false, // Clean
        _ => unreachable!(),
    };

    let subcommand = match kind {
        0 => Subcommand::TestAll,
        1 => Subcommand::Build,
        2 => Subcommand::Fmt,
        3 => Subcommand::RunBinary {
            entry_name: entry,
            bin_name: part,
            args: &[],
        },
        4 => Subcommand::Test {
            entry_name: entry,
            test_name: part,
        },
        5 => Subcommand::Clean,
        _ => unreachable!(),
    };

    let result = sub_info.try_check_subcommand(&mut interner, &subcommand);
    assert_eq!(result.is_ok(), expected);
});
