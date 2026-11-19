#![no_main]

use libfuzzer_sys::arbitrary::unstructured::Unstructured;
use libfuzzer_sys::fuzz_target;

use std::path::Path;
use std::sync::OnceLock;
use zxbs::config::Config;
use zxbs::hash::{Hash, HashMap, hash_bytes, mix_hashes};
use zxbs::interner::Interner;
use zxbs::plan::{DeepHashes, construct_deep_hashes};
use zxbs::scan::BuildInfo;

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
        build_lock_path: Path::new("/lock"),
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

fn random_hash(fuzz_input: &mut Unstructured) -> Option<Hash> {
    fuzz_input.arbitrary::<Hash>().ok()
}

#[derive(Clone, Copy)]
enum FileKind {
    Module,
    Lib,
    Bin,
    Test,
}

#[derive(Clone)]
struct FileSpec {
    path: String,
    kind: FileKind,
    entry: usize,
    hash: Hash,
}

#[derive(Clone)]
struct EntrySpec {
    name: String,
    deps: Vec<usize>,
    // The first module is the root module and is named after the entry.
    modules: Vec<String>,
    // For each module part, the indices of the parts it depends on.
    module_deps: Vec<Vec<usize>>,
    libs: Vec<String>,
    bins: Vec<String>,
    tests: Vec<String>,
}

#[derive(Clone)]
struct Spec {
    entries: Vec<EntrySpec>,
    files: Vec<FileSpec>,
}

fn make_spec(fuzz_input: &mut Unstructured) -> Option<Spec> {
    let num_entries = count(fuzz_input, 5) + 1;
    let mut entries = Vec::with_capacity(num_entries);
    let mut files = Vec::new();

    for entry_idx in 0..num_entries {
        let name = format!("e{}", entry_idx);

        let mut deps = Vec::new();
        for dep in 0..entry_idx {
            if coin(fuzz_input) {
                deps.push(dep);
            }
        }

        let num_modules = count(fuzz_input, 4) + 1;
        let mut modules = Vec::with_capacity(num_modules);
        modules.push(name.clone());
        for part in 1..num_modules {
            modules.push(format!("s{}", part));
        }

        let mut module_deps = Vec::with_capacity(num_modules);
        // Root depends on every submodule.
        module_deps.push((1..num_modules).collect());
        for part in 1..num_modules {
            let mut part_deps = Vec::new();
            for dep in 1..part {
                if coin(fuzz_input) {
                    part_deps.push(dep);
                }
            }
            module_deps.push(part_deps);
        }

        let num_libs = count(fuzz_input, 3);
        let libs: Vec<String> = (0..num_libs).map(|i| format!("l{}", i)).collect();

        let num_bins = count(fuzz_input, 3);
        let bins: Vec<String> = (0..num_bins).map(|i| format!("b{}", i)).collect();

        let num_tests = count(fuzz_input, 3);
        let tests: Vec<String> = (0..num_tests).map(|i| format!("t{}", i)).collect();

        for part in modules.iter() {
            let path = format!("/src/{}/mod/{}.cppm", name, part);
            let hash = mix_hashes(hash_bytes(path.as_bytes()), random_hash(fuzz_input)?);
            files.push(FileSpec {
                path,
                kind: FileKind::Module,
                entry: entry_idx,
                hash,
            });
        }
        for part in libs.iter() {
            let path = format!("/src/{}/lib/{}.cc", name, part);
            let hash = mix_hashes(hash_bytes(path.as_bytes()), random_hash(fuzz_input)?);
            files.push(FileSpec {
                path,
                kind: FileKind::Lib,
                entry: entry_idx,
                hash,
            });
        }
        for part in bins.iter() {
            let path = format!("/src/{}/bin/{}.cc", name, part);
            let hash = mix_hashes(hash_bytes(path.as_bytes()), random_hash(fuzz_input)?);
            files.push(FileSpec {
                path,
                kind: FileKind::Bin,
                entry: entry_idx,
                hash,
            });
        }
        for part in tests.iter() {
            let path = format!("/src/{}/test/{}.cc", name, part);
            let hash = mix_hashes(hash_bytes(path.as_bytes()), random_hash(fuzz_input)?);
            files.push(FileSpec {
                path,
                kind: FileKind::Test,
                entry: entry_idx,
                hash,
            });
        }

        entries.push(EntrySpec {
            name,
            deps,
            modules,
            module_deps,
            libs,
            bins,
            tests,
        });
    }

    Some(Spec { entries, files })
}

fn make_info(spec: &Spec, interner: &mut Interner) -> BuildInfo {
    let names: Vec<_> = spec
        .entries
        .iter()
        .map(|entry| interner.insert(&entry.name))
        .collect();
    let map: HashMap<_, u32> = names
        .iter()
        .enumerate()
        .map(|(idx, &name)| (name, u32::try_from(idx).unwrap()))
        .collect();

    let module: Vec<Vec<_>> = spec
        .entries
        .iter()
        .map(|entry| {
            entry
                .modules
                .iter()
                .map(|part| interner.insert(part))
                .collect()
        })
        .collect();

    let mod_dep_matrix: Vec<Vec<bool>> = spec
        .entries
        .iter()
        .map(|entry| {
            let num_parts = entry.modules.len();
            let mut matrix = vec![false; num_parts * num_parts];
            for (part, deps) in entry.module_deps.iter().enumerate() {
                for &dep in deps.iter() {
                    matrix[part * num_parts + dep] = true;
                }
            }
            matrix
        })
        .collect();

    let deps: Vec<Vec<_>> = spec
        .entries
        .iter()
        .map(|entry| entry.deps.iter().map(|&dep| names[dep]).collect())
        .collect();

    let lib: Vec<Vec<_>> = spec
        .entries
        .iter()
        .map(|entry| {
            entry
                .libs
                .iter()
                .map(|part| interner.insert(part))
                .collect()
        })
        .collect();
    let bin: Vec<Vec<_>> = spec
        .entries
        .iter()
        .map(|entry| {
            entry
                .bins
                .iter()
                .map(|part| interner.insert(part))
                .collect()
        })
        .collect();
    let test: Vec<Vec<_>> = spec
        .entries
        .iter()
        .map(|entry| {
            entry
                .tests
                .iter()
                .map(|part| interner.insert(part))
                .collect()
        })
        .collect();

    let mut basic_hash = HashMap::default();
    for file in spec.files.iter() {
        let key = interner.insert(&file.path);
        assert!(basic_hash.insert(key, file.hash).is_none());
    }

    BuildInfo {
        map,
        name: names,
        module,
        mod_dep_matrix,
        lib,
        bin,
        test,
        deps,
        basic_hash,
    }
}

fn snapshot(hashes: &DeepHashes) -> Vec<Hash> {
    let mut out = Vec::new();
    out.extend_from_slice(&hashes.entry_pcm);
    out.extend_from_slice(&hashes.entry_lib_archive);
    for list in hashes.module_pcm.iter() {
        out.extend_from_slice(list);
    }
    for list in hashes.module_object.iter() {
        out.extend_from_slice(list);
    }
    for list in hashes.lib_object.iter() {
        out.extend_from_slice(list);
    }
    for list in hashes.bin_object.iter() {
        out.extend_from_slice(list);
    }
    for list in hashes.bin_exe.iter() {
        out.extend_from_slice(list);
    }
    for list in hashes.test_object.iter() {
        out.extend_from_slice(list);
    }
    for list in hashes.test_exe.iter() {
        out.extend_from_slice(list);
    }
    for list in hashes.test_stamp.iter() {
        out.extend_from_slice(list);
    }
    out
}

fn build_hashes(spec: &Spec) -> (Vec<Hash>, DeepHashes) {
    let mut interner = Interner::new();
    let info = make_info(spec, &mut interner);
    let hashes = construct_deep_hashes(config(), &mut interner, &info, 0);
    (snapshot(&hashes), hashes)
}

fuzz_target!(|data: &[u8]| {
    let mut fuzz_input = Unstructured::new(data);
    let Some(spec) = make_spec(&mut fuzz_input) else {
        return;
    };

    let (base_snapshot, base) = build_hashes(&spec);

    // Computing the same spec twice must produce the same hashes.
    let (again_snapshot, _) = build_hashes(&spec);
    assert_eq!(base_snapshot, again_snapshot);

    // Dependency and file order must not matter.
    let mut shuffled = spec.clone();
    for entry in shuffled.entries.iter_mut() {
        entry.deps.reverse();
    }
    shuffled.files.reverse();
    let (shuffled_snapshot, _) = build_hashes(&shuffled);
    assert_eq!(base_snapshot, shuffled_snapshot);

    // Changing one source file must invalidate exactly the artifacts that
    // depend on it.
    let file_idx = count(&mut fuzz_input, spec.files.len());
    let mut changed = spec.clone();
    changed.files[file_idx].hash = changed.files[file_idx].hash.wrapping_add(1);

    let (changed_snapshot, after) = build_hashes(&changed);
    assert_ne!(base_snapshot, changed_snapshot);

    let file = &spec.files[file_idx];
    let entry = file.entry;
    match file.kind {
        FileKind::Module => {
            assert_ne!(base.module_pcm[entry], after.module_pcm[entry]);
            assert_ne!(base.module_object[entry], after.module_object[entry]);
            assert_ne!(base.entry_pcm[entry], after.entry_pcm[entry]);
        }
        FileKind::Lib => {
            assert_eq!(base.entry_pcm[entry], after.entry_pcm[entry]);
            assert_ne!(base.lib_object[entry], after.lib_object[entry]);
            assert_ne!(
                base.entry_lib_archive[entry],
                after.entry_lib_archive[entry]
            );
        }
        FileKind::Bin => {
            assert_eq!(base.entry_pcm[entry], after.entry_pcm[entry]);
            assert_ne!(base.bin_object[entry], after.bin_object[entry]);
            assert_ne!(base.bin_exe[entry], after.bin_exe[entry]);
        }
        FileKind::Test => {
            assert_eq!(base.entry_pcm[entry], after.entry_pcm[entry]);
            assert_ne!(base.test_object[entry], after.test_object[entry]);
            assert_ne!(base.test_exe[entry], after.test_exe[entry]);
            assert_ne!(base.test_stamp[entry], after.test_stamp[entry]);
        }
    }
});
