use std::collections::HashMap;
use std::path::Path;

use smol::stream::StreamExt;

use crate::config::Config;
use crate::interner::{Interner, Str};

#[derive(Default)]
pub struct BuildInfo {
    pub map: HashMap<Str, u32>,
    pub name: Vec<Str>,
    pub module: Vec<Vec<Str>>,
    // matrix of mod_count * mod_count.
    // matrix[i * mod_count + j] is true if mod no i depends on mod no j.
    pub mod_dep_matrix: Vec<Vec<bool>>,
    pub lib: Vec<Vec<Str>>,
    pub bin: Vec<Vec<Str>>,
    pub test: Vec<Vec<Str>>,
    pub deps: Vec<Vec<Str>>,
}

pub fn parse_p1689(contents: &[u8]) -> Vec<String> {
    let root = serde_json::from_slice::<serde_json::Value>(contents).unwrap();
    let root = root.as_object().unwrap();

    let mut deps = Vec::new();

    if let Some(rules) = root.get("rules") {
        for rule in rules.as_array().unwrap().iter() {
            let rule = rule.as_object().unwrap();
            let reqs = match rule.get("requires") {
                Some(req) => req,
                None => continue,
            };
            for p in reqs.as_array().unwrap().iter() {
                let name = p
                    .as_object()
                    .unwrap()
                    .get("logical-name")
                    .unwrap()
                    .as_str()
                    .unwrap();
                deps.push(name.to_owned());
            }
        }
    }

    deps
}

async fn scan_deps(config: &Config, path: &Path) -> Vec<String> {
    let mut cmd = smol::process::Command::new(config.clang_scan_deps);
    cmd.arg("-format=p1689")
        .arg("--")
        .arg(config.cxx)
        .args(config.cxxflags)
        .arg("-c")
        .arg(path);
    let output = match cmd.output().await {
        Ok(out) => out,
        Err(e) => {
            panic!(
                "Failed to run scan deps command for {}: {}",
                path.display(),
                e
            );
        }
    };
    if !output.status.success() {
        panic!(
            "Failed clang_scan_deps for {}:\n{}",
            path.display(),
            str::from_utf8(&output.stderr).unwrap()
        );
    }

    parse_p1689(output.stdout.as_slice())
}

pub fn parse_deps_file(interner: &mut Interner, contents: &str) -> Vec<Str> {
    let mut indx = Vec::new();
    for line in contents.trim().split('\n') {
        let line = line.trim();
        if !line.is_empty() {
            check_name(line);
            indx.push(interner.insert(line));
        }
    }
    indx
}

async fn read_deps_file(interner: &mut Interner, path: &Path) -> Vec<Str> {
    let contents = String::from_utf8(smol::fs::read(path).await.unwrap_or_else(|e| {
        panic!(
            "failed to read deps file at: {}. Error={}",
            path.display(),
            e
        )
    }))
    .unwrap();
    parse_deps_file(interner, contents.as_str())
}

fn check_name(name_str: &str) {
    let name = name_str.as_bytes();
    assert!(!name.is_empty(), "invalid name: {}", name_str);
    assert!(
        name[0] >= b'a' && name[0] <= b'z',
        "invalid name: {}",
        name_str
    );
    for &byte in name {
        let is_char = byte >= b'a' && byte <= b'z';
        let is_digit = byte.is_ascii_digit();
        let is_underscore = byte == b'_';
        assert!(
            is_char || is_digit || is_underscore,
            "invalid name: {}",
            name_str,
        );
    }
}

pub fn parse_file_name<'name>(
    path: &Path,
    expected_extension: &str,
    file_name: &'name str,
) -> &'name str {
    let parts: Vec<_> = file_name.split(".").collect();
    assert_eq!(
        parts.len(),
        2,
        "invalid file name: {}, expected extension: {}. Path={}",
        file_name,
        expected_extension,
        path.display(),
    );
    let name = parts[0];
    let extension = parts[1];

    check_name(name);

    assert_eq!(
        expected_extension,
        extension,
        "unexpected extension on filename: {}, expected extension: {}. Path={}",
        file_name,
        expected_extension,
        path.display(),
    );

    name
}

async fn scan_subfolder(
    expected_extension: &str,
    interner: &mut Interner,
    path: &Path,
) -> Vec<Str> {
    let mut dir = smol::fs::read_dir(path).await.unwrap();
    let mut names = Vec::new();
    while let Some(file) = dir.next().await {
        let file = file.unwrap();
        let name = parse_file_name(
            &file.path(),
            expected_extension,
            file.file_name().to_str().unwrap(),
        )
        .to_owned();
        check_name(&name);
        let name_index = interner.insert(&name);
        names.push(name_index);
    }
    names
}

pub async fn scan(config: &Config, interner: &mut Interner) -> BuildInfo {
    let mut dir = smol::fs::read_dir(config.src_path).await.unwrap();

    let mut info = BuildInfo::default();

    let mut entry_idx: u32 = 0;
    while let Some(entry) = dir.next().await {
        let entry = entry.unwrap();
        let path = entry.path();
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_str().unwrap();
        check_name(entry_name);
        let entry_name = interner.insert(entry_name);

        // name of entries should be unique
        assert!(info.map.insert(entry_name, entry_idx).is_none());

        info.name.push(entry_name);

        let deps = read_deps_file(interner, &path.join("deps")).await;

        let mod_parts = scan_subfolder("cppm", interner, &path.join("mod")).await;
        let n_mod_parts = mod_parts.len();

        // has to have root module
        assert!(
            mod_parts.contains(&entry_name),
            "root module of entry {} not found",
            interner.get(entry_name),
        );

        let mod_path = path.join("mod");

        let mut mod_dep_matrix = vec![false; n_mod_parts * n_mod_parts];
        for (part_pos, &part_name) in mod_parts.iter().enumerate() {
            let mod_dep_names = scan_deps(
                config,
                &mod_path.join(format!("{}.cppm", interner.get(part_name))),
            )
            .await;
            for mod_dep_name in mod_dep_names.iter() {
                if mod_dep_name.contains(':') {
                    let parts: Vec<_> = mod_dep_name.split(':').collect();
                    assert_eq!(parts.len(), 2);
                    check_name(parts[0]);
                    check_name(parts[1]);
                    let part0 = interner.insert(parts[0]);
                    let part1 = interner.insert(parts[1]);

                    assert_eq!(
                        part0,
                        entry_name,
                        "part of entry {} doesn't define a module under it. It instead defines under {}",
                        interner.get(entry_name),
                        interner.get(part0),
                    );
                    assert_ne!(
                        part1,
                        part_name,
                        "part {} of entry {} doesn't define expected module {}:{}",
                        interner.get(part_name),
                        interner.get(entry_name),
                        interner.get(entry_name),
                        interner.get(part_name),
                    );

                    let dep_pos = mod_parts
                        .iter()
                        .position(|nidx| *nidx == part1)
                        .unwrap_or_else(||
                            panic!(
                                "dependency with name {} of part with name {} not found in entry with name {}",
                                mod_dep_name, interner.get(part_name), interner.get(entry_name),
                            )
                        );

                    mod_dep_matrix[part_pos * n_mod_parts + dep_pos] = true;
                    assert!(
                        !mod_dep_matrix[dep_pos * n_mod_parts + part_pos],
                        "cyclic dependency between {} and {} in entry {}",
                        interner.get(part_name),
                        mod_dep_name,
                        interner.get(entry_name),
                    );
                } else {
                    let mod_dep_name_idx = interner.insert(mod_dep_name);
                    let is_direct_dep = deps.contains(&mod_dep_name_idx);
                    assert!(
                        is_direct_dep,
                        "Module file {}:{} depends on {} but it isn't listed in the deps file of {}",
                        interner.get(entry_name),
                        interner.get(part_name),
                        mod_dep_name,
                        interner.get(entry_name),
                    );
                }
            }
        }

        info.deps.push(deps);
        info.mod_dep_matrix.push(mod_dep_matrix);
        info.module.push(mod_parts);

        let lib = scan_subfolder("cc", interner, &path.join("lib")).await;
        info.lib.push(lib);

        let test = scan_subfolder("cc", interner, &path.join("test")).await;
        info.test.push(test);

        let bin = scan_subfolder("cc", interner, &path.join("bin")).await;
        info.bin.push(bin);

        entry_idx += 1;
    }

    info
}
