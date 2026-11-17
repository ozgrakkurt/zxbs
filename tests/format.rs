use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use zxbs::format::format;
use zxbs::thread_pool::ThreadPool;

mod tmp_dir;
use tmp_dir::TempDir;

fn write_file(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

// Format commands are recorded by a fake clang-format script so the test does
// not need clang-format installed and does not modify any source file.
fn make_fake_clang_format(folder: &TempDir, log_path: &Path) -> &'static str {
    let script_path = folder.join("fake-clang-format");

    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nfor last; do :; done\nprintf '%s\\n' \"$last\" >> '{}'\n",
            log_path.display(),
        ),
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    Box::leak(script_path.to_str().unwrap().to_owned().into_boxed_str())
}

#[test]
fn test_format_discovers_source_files() {
    let folder = TempDir::new();
    let src = folder.join("src");

    // An entry that uses every source folder.
    write_file(src.join("full/mod/full.cppm"), "export module full;\n");
    write_file(src.join("full/mod/part.cppm"), "export module full:part;\n");
    write_file(src.join("full/lib/impl.cc"), "import full;\n");
    write_file(src.join("full/bin/tool.cc"), "import full;\n");
    write_file(src.join("full/test/check.cc"), "import full;\n");
    write_file(src.join("full/deps"), "");

    // Module-only entries have no lib, bin or test folders. `fmt` must not
    // require the folders that `scan` treats as optional.
    write_file(src.join("solo/mod/solo.cppm"), "export module solo;\n");

    // An entry with only a lib folder.
    write_file(src.join("libonly/lib/only.cc"), "");

    // Files that are not part of the source structure are ignored by `fmt`.
    write_file(src.join("solo/notes.txt"), "not a source file\n");

    let log_path = folder.join("formatted.log");
    let clang_format = make_fake_clang_format(&folder, &log_path);

    let thread_pool = ThreadPool::new(8);
    format(&thread_pool, clang_format, &src);

    let mut formatted: Vec<String> = std::fs::read_to_string(&log_path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    formatted.sort_unstable();

    let mut expected: Vec<String> = [
        src.join("full/mod/full.cppm"),
        src.join("full/mod/part.cppm"),
        src.join("full/lib/impl.cc"),
        src.join("full/bin/tool.cc"),
        src.join("full/test/check.cc"),
        src.join("solo/mod/solo.cppm"),
        src.join("libonly/lib/only.cc"),
    ]
    .into_iter()
    .map(|path| path.to_str().unwrap().to_owned())
    .collect();
    expected.sort_unstable();

    assert_eq!(formatted, expected);
}

// `fmt` used to go through the normal build path, which deleted the build
// output directory before planning. It must be a standalone operation.
#[test]
fn test_fmt_does_not_touch_build_dir() {
    let folder = TempDir::new();
    let src = folder.join("src");
    write_file(src.join("app/mod/app.cppm"), "export module app;\n");

    let build_dir = folder.join("build");
    let marker = build_dir.join("out").join("marker");
    write_file(&marker, "keep me");

    let log_path = folder.join("formatted.log");
    let clang_format = make_fake_clang_format(&folder, &log_path);

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_zxbs"))
        .arg(format!("--src-dir={}", src.display()))
        .arg(format!("--build-dir={}", build_dir.display()))
        .arg(format!("--clang-format={}", clang_format))
        .arg("fmt")
        .status()
        .unwrap();

    assert!(status.success());
    assert!(marker.exists(), "fmt must not delete the build output");

    let formatted = std::fs::read_to_string(&log_path).unwrap();
    let expected = src.join("app/mod/app.cppm");
    assert!(
        formatted
            .lines()
            .any(|line| line == expected.to_str().unwrap()),
        "fmt did not format the root module: {formatted:?}",
    );
}
