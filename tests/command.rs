use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use zxbs::command::{
    ArchiveLib, CacheLoad, CacheStore, Command, CommandKind, CompileModule, CompileToObject,
    LinkBinary, RunBinary, RunTest, TestCmd,
};
use zxbs::hash::{Hash, hash_bytes};

mod tmp_dir;
use tmp_dir::TempDir;

// The command structs hold `&'static str` paths, so the tests leak the strings
// they put in them. The leaks are per-test and tiny.
fn static_str(path: &std::path::Path) -> &'static str {
    Box::leak(path.to_str().unwrap().to_owned().into_boxed_str())
}

// One command of every kind with recognizable fields.
fn make_commands() -> Vec<(Command, CommandKind, &'static str)> {
    let test_cmd = TestCmd {
        finished: Arc::new(Mutex::new(HashSet::default())),
        inputs: Vec::new(),
        outputs: Vec::new(),
    };

    let deep: Hash = 0x1234;

    vec![
        (
            Command::CompileModule(CompileModule {
                cxx: "clang++",
                cxxflags: &["-O2"],
                input: "in.cppm",
                object_output: "out.o",
                pcm_output: "out.pcm",
            }),
            CommandKind::CompileModule,
            "<cxx> <cxxflags> in.cppm -fmodule-output=out.pcm -c -o out.o",
        ),
        (
            Command::CompileToObject(CompileToObject {
                cxx: "clang++",
                cxxflags: &["-O2"],
                input: "in.cc",
                output: "out.o",
            }),
            CommandKind::CompileToObject,
            "<cxx> <cxxflags> in.cc -c -o out.o",
        ),
        (
            Command::ArchiveLib(ArchiveLib {
                ar: "llvm-ar",
                output: "out.a",
                inputs: vec!["first.o", "second.o"],
            }),
            CommandKind::ArchiveLib,
            "<ar> crs out.a first.o second.o",
        ),
        (
            Command::LinkBinary(LinkBinary {
                cxx: "clang++",
                ldflags: &["-static"],
                output: "bin",
                object: "main.o",
                deps: &["lib1.a", "lib2.a"],
            }),
            CommandKind::LinkBinary,
            "<cxx> <ldflags> -o bin main.o lib1.a lib2.a",
        ),
        (
            Command::RunBinary(RunBinary {
                path: "bin",
                args: &["--flag"],
            }),
            CommandKind::RunBinary,
            "RUN_BINARY bin",
        ),
        (
            Command::RunTest(RunTest {
                path: "test_bin",
                stamp_hash: 7,
                stamp_path: "stamp",
            }),
            CommandKind::RunTest,
            "RUN_TEST test_bin",
        ),
        (
            Command::CacheLoad(CacheLoad {
                cache_files_path: "cache",
                output_path: "out",
                deep_hash: deep,
                expected_hash: 0,
            }),
            CommandKind::CacheLoad,
            "CACHE_LOAD 1234",
        ),
        (
            Command::CacheStore(CacheStore {
                cache_files_path: "cache",
                file_path: "out",
                deep_hash: deep,
            }),
            CommandKind::CacheStore,
            "CACHE_STORE 1234",
        ),
        (
            Command::TestCmd(test_cmd),
            CommandKind::TestCmd,
            "Test Command",
        ),
    ]
}

// Every command reports its own kind and a useful human readable
// representation.
#[test]
fn test_command_kind_and_print() {
    for (command, kind, expected_print) in make_commands() {
        assert_eq!(command.kind(), kind);
        assert_eq!(command.print(), expected_print);
    }
}

// Storing a file to the cache and loading it back must reproduce the exact
// contents. The file is copied (not rewritten) so the executable bit of a
// linked binary survives the roundtrip.
#[test]
fn test_cache_store_and_load_roundtrip() {
    let folder = TempDir::new();
    let cache_dir = folder.join("cache");
    std::fs::create_dir_all(cache_dir.join("wal")).unwrap();

    let content: &[u8] = b"cached artifact contents";
    let deep: Hash = 0x1234;

    let source = folder.join("artifact");
    std::fs::write(&source, content).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();

    CacheStore {
        cache_files_path: static_str(&cache_dir),
        file_path: static_str(&source),
        deep_hash: deep,
    }
    .run();

    // Storing writes the cached file and a wal entry for crash recovery.
    assert!(cache_dir.join(format!("{:x}", deep)).exists());
    assert!(cache_dir.join("wal").join(format!("{:x}", deep)).exists());

    // Load the file back after the original is gone.
    std::fs::remove_file(&source).unwrap();

    CacheLoad {
        cache_files_path: static_str(&cache_dir),
        output_path: static_str(&source),
        deep_hash: deep,
        expected_hash: hash_bytes(content),
    }
    .run();

    assert_eq!(std::fs::read(&source).unwrap(), content);

    let mode = std::fs::metadata(&source).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "executable bit must survive the cache");
}

// A cached file that doesn't match the recorded hash means the cache is
// broken; loading must not silently accept it.
#[test]
#[should_panic(expected = "assertion failed")]
fn test_cache_load_rejects_a_corrupted_cache() {
    let folder = TempDir::new();
    let cache_dir = folder.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();

    let deep: Hash = 0x1234;
    std::fs::write(cache_dir.join(format!("{:x}", deep)), b"tampered contents").unwrap();

    CacheLoad {
        cache_files_path: static_str(&cache_dir),
        output_path: static_str(&folder.join("out")),
        deep_hash: deep,
        expected_hash: hash_bytes(b"original contents"),
    }
    .run();
}

// A command whose process exits with a failure status, standing in for a
// broken toolchain. `/bin/false` is used instead of a freshly written
// script: exec'ing a file that was just written is racy when other tests
// spawn processes concurrently, because the spawned child briefly inherits
// the open write fd and the kernel reports the file as busy.
fn failing_tool() -> &'static str {
    "/bin/false"
}

#[test]
#[should_panic(expected = "Compile module (.cppm) Failed")]
fn test_compile_module_failure_panics() {
    CompileModule {
        cxx: failing_tool(),
        cxxflags: &[],
        input: "in.cppm",
        object_output: "out.o",
        pcm_output: "out.pcm",
    }
    .run();
}

#[test]
#[should_panic(expected = "Compile to .o Failed")]
fn test_compile_to_object_failure_panics() {
    CompileToObject {
        cxx: failing_tool(),
        cxxflags: &[],
        input: "in.cc",
        output: "out.o",
    }
    .run();
}

#[test]
#[should_panic(expected = "Archiving a library failed")]
fn test_archive_lib_failure_panics() {
    ArchiveLib {
        ar: failing_tool(),
        output: "out.a",
        inputs: vec!["first.o"],
    }
    .run();
}

#[test]
#[should_panic(expected = "Linking a binary failed")]
fn test_link_binary_failure_panics() {
    LinkBinary {
        cxx: failing_tool(),
        ldflags: &[],
        output: "bin",
        object: "main.o",
        deps: &[],
    }
    .run();
}

// A failing command that also prints to stdout must report the output before
// panicking. `sh -c` runs the noise script, ignoring the extra compile
// arguments passed after it.
#[test]
#[should_panic(expected = "Compile module (.cppm) Failed")]
fn test_compile_module_reports_stdout_noise() {
    CompileModule {
        cxx: "sh",
        cxxflags: &["-c", "echo some stdout noise; exit 1"],
        input: "in.cppm",
        object_output: "out.o",
        pcm_output: "out.pcm",
    }
    .run();
}

#[test]
#[should_panic(expected = "Failed to run command")]
fn test_missing_tool_panics() {
    CompileToObject {
        cxx: "not-a-real-tool-xyz",
        cxxflags: &[],
        input: "in.cc",
        output: "out.o",
    }
    .run();
}
