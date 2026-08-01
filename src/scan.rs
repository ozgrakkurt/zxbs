use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::task::JoinSet;

use crate::config::Config;
use crate::interner::{Interner, Str};

struct EntryInfo {
    name: String,
    module: Vec<String>,
    mod_dep_matrix: Vec<bool>,
    lib: Vec<String>,
    bin: Vec<String>,
    test: Vec<String>,
    deps: Vec<String>,
}

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

impl BuildInfo {
    fn push_entry_info(&mut self, interner: &mut Interner, entry_info: EntryInfo) {
        let index = u32::try_from(self.name.len()).unwrap();
        let name = interner.insert(&entry_info.name);
        assert!(self.map.insert(name, index).is_none());
        self.name.push(name);
        self.module.push(
            entry_info
                .module
                .iter()
                .map(|s| interner.insert(s))
                .collect(),
        );
        self.mod_dep_matrix.push(entry_info.mod_dep_matrix);
        self.lib
            .push(entry_info.lib.iter().map(|s| interner.insert(s)).collect());
        self.bin
            .push(entry_info.bin.iter().map(|s| interner.insert(s)).collect());
        self.test
            .push(entry_info.test.iter().map(|s| interner.insert(s)).collect());
        self.deps
            .push(entry_info.deps.iter().map(|s| interner.insert(s)).collect());
    }
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

async fn scan_deps(config: &Config, path: PathBuf) -> Vec<String> {
    let mut cmd = tokio::process::Command::new(config.clang_scan_deps);
    cmd.arg("-format=p1689")
        .arg("--")
        .arg(config.cxx)
        .args(config.cxxflags)
        .arg("-c")
        .arg(&path);
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

pub fn parse_deps_file(contents: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in contents.trim().split('\n') {
        let line = line.trim();
        if !line.is_empty() {
            check_name(line);
            deps.push(line.to_owned());
        }
    }
    deps
}

async fn read_deps_file(path: PathBuf) -> Vec<String> {
    let contents = String::from_utf8(tokio::fs::read(&path).await.unwrap_or_else(|e| {
        panic!(
            "failed to read deps file at: {}. Error={}",
            path.display(),
            e
        )
    }))
    .unwrap();
    parse_deps_file(contents.as_str())
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

async fn scan_subfolder(expected_extension: &'static str, path: PathBuf) -> Vec<String> {
    let mut dir = tokio::fs::read_dir(&path).await.unwrap();
    let mut names = Vec::new();
    while let Some(file) = dir.next_entry().await.unwrap() {
        let name = parse_file_name(
            &file.path(),
            expected_extension,
            file.file_name().to_str().unwrap(),
        )
        .to_owned();
        check_name(&name);
        names.push(name);
    }
    names
}

async fn scan_entry(config: &'static Config, path: &Path) -> EntryInfo {
    let entry_name = path.file_name().unwrap();
    let entry_name = entry_name.to_str().unwrap().to_owned();
    check_name(&entry_name);

    let deps = tokio::spawn(read_deps_file(path.join("deps")));

    let mod_parts = tokio::spawn(scan_subfolder("cppm", path.join("mod")));

    let lib = tokio::task::spawn(scan_subfolder("cc", path.join("lib")));
    let test = tokio::task::spawn(scan_subfolder("cc", path.join("test")));
    let bin = tokio::task::spawn(scan_subfolder("cc", path.join("bin")));

    let deps = deps.await.unwrap();
    let mod_parts = mod_parts.await.unwrap();
    let n_mod_parts = mod_parts.len();

    // has to have root module
    assert!(
        mod_parts.contains(&entry_name),
        "root module of entry {} not found",
        entry_name,
    );

    let mod_path = path.join("mod");

    let concurrency = 16;
    let mut join_set = JoinSet::new();
    let mut mod_dep_names = vec![Vec::new(); mod_parts.len()];
    for (pos, part_name) in mod_parts.iter().enumerate() {
        if join_set.len() >= concurrency {
            let (idx, deps) = join_set.join_next().await.unwrap().unwrap();
            mod_dep_names[idx] = deps;
        }
        let part_name = part_name.to_owned();
        let cppm_path = mod_path.join(format!("{}.cppm", part_name));
        join_set.spawn(async move {
            let deps = scan_deps(config, cppm_path).await;
            (pos, deps)
        });
    }
    while let Some(res) = join_set.join_next().await {
        let (idx, deps) = res.unwrap();
        mod_dep_names[idx] = deps;
    }

    let mut mod_dep_matrix = vec![false; n_mod_parts * n_mod_parts];
    for (part_pos, part_name) in mod_parts.iter().enumerate() {
        let mod_dep_names = &mod_dep_names[part_pos];
        for mod_dep_name in mod_dep_names.iter() {
            if mod_dep_name.contains(':') {
                let parts: Vec<_> = mod_dep_name.split(':').collect();
                assert_eq!(parts.len(), 2);
                check_name(parts[0]);
                check_name(parts[1]);
                let part0 = parts[0];
                let part1 = parts[1];

                assert_eq!(
                    part0, entry_name,
                    "part of entry {} doesn't define a module under it. It instead defines under {}",
                    entry_name, part0,
                );
                assert_ne!(
                    part1, part_name,
                    "part {} of entry {} doesn't define expected module {}:{}",
                    part_name, entry_name, entry_name, part_name,
                );

                let dep_pos = mod_parts
                        .iter()
                        .position(|nidx| *nidx == part1)
                        .unwrap_or_else(||
                            panic!(
                                "dependency with name {} of part with name {} not found in entry with name {}",
                                mod_dep_name, part_name, entry_name,
                            )
                        );

                mod_dep_matrix[part_pos * n_mod_parts + dep_pos] = true;
                assert!(
                    !mod_dep_matrix[dep_pos * n_mod_parts + part_pos],
                    "cyclic dependency between {} and {} in entry {}",
                    part_name,
                    mod_dep_name,
                    entry_name,
                );
            } else {
                let is_direct_dep = deps.contains(mod_dep_name);
                assert!(
                    is_direct_dep,
                    "Module file {}:{} depends on {} but it isn't listed in the deps file of {}",
                    entry_name, part_name, mod_dep_name, entry_name,
                );
            }
        }
    }

    let lib = lib.await.unwrap();
    let test = test.await.unwrap();
    let bin = bin.await.unwrap();

    EntryInfo {
        name: entry_name,
        module: mod_parts,
        mod_dep_matrix,
        lib,
        bin,
        test,
        deps,
    }
}

pub async fn scan(config: &'static Config, interner: &mut Interner) -> BuildInfo {
    let mut dir = tokio::fs::read_dir(config.src_path).await.unwrap();

    let mut info = BuildInfo::default();

    let mut join_set = JoinSet::new();
    let concurrency = 16;

    while let Some(entry) = dir.next_entry().await.unwrap() {
        if join_set.len() >= concurrency {
            let entry_info = join_set.join_next().await.unwrap().unwrap();
            info.push_entry_info(interner, entry_info);
        }
        join_set.spawn(async move { scan_entry(config, &entry.path()).await });
    }

    while let Some(res) = join_set.join_next().await {
        let entry_info = res.unwrap();
        info.push_entry_info(interner, entry_info);
    }

    info
}
