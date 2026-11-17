use std::io::ErrorKind as IoErrorKind;
use std::path::Path;

use crate::common::{check_name, try_parse_file_name};
use crate::thread_pool::{JoinSet, ThreadPool};

fn format_file(clang_format: &str, path: &str) {
    let output = match std::process::Command::new(clang_format)
        .arg("-i")
        .arg("--style={BasedOnStyle: LLVM, IndentWidth: 4}")
        .arg(path)
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            panic!("Failed to run format on `{}`.\nError = {:?}\n", path, e,);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).unwrap();
        panic!(
            "Running clang format on `{}` failed.\nStatus: {}\nStdErr:\n{}\n",
            path, output.status, stderr,
        );
    }

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        eprintln!(
            "STDOUT of running clang format on `{}`:\n{}",
            path,
            stdout.as_str()
        );
    }
}

fn scan_subfolder(extension: &str, folder_path: &Path) -> Vec<String> {
    let dir = match std::fs::read_dir(folder_path) {
        Ok(dir) => dir,
        Err(e) => {
            if e.kind() != IoErrorKind::NotFound {
                panic!(
                    "Failed to read_dir on '{}'. Error: {}",
                    folder_path.display(),
                    e
                );
            } else {
                return Vec::new();
            }
        }
    };

    let mut out = Vec::new();

    for entry in dir {
        let entry = entry.unwrap();

        let file_type = entry.file_type().unwrap();
        let file_path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_str().unwrap();

        if !file_type.is_file() {
            panic!(
                "Found unexpected kind of file while scanning subfolder. `{}`",
                file_path.display()
            );
        }

        let name = try_parse_file_name(extension, file_name).unwrap_or_else(|e| {
            panic!(
                "Failed to parse the file name at: `{}`. Error = {}",
                file_path.display(),
                e
            );
        });

        if check_name(name).is_err() {
            panic!("Invalid name at: `{}`", file_path.display());
        }

        out.push(file_path.to_str().unwrap().to_owned());
    }

    out
}

pub fn format(thread_pool: &ThreadPool, clang_format: &'static str, src_path: &Path) {
    let mut scan_subfolder_join_set: JoinSet<Vec<String>> = JoinSet::new();

    let dir = std::fs::read_dir(src_path).unwrap();

    for entry in dir {
        let entry = entry.unwrap();

        let file_type = entry.file_type().unwrap();
        let path = entry.path();

        if !file_type.is_dir() {
            panic!(
                "Found unexpected file at: `{}` while scanning.",
                path.display()
            );
        }

        let p = path.join("mod");
        scan_subfolder_join_set.spawn(thread_pool, move || scan_subfolder("cppm", &p));
        let p = path.join("lib");
        scan_subfolder_join_set.spawn(thread_pool, move || scan_subfolder("cc", &p));
        let p = path.join("bin");
        scan_subfolder_join_set.spawn(thread_pool, move || scan_subfolder("cc", &p));
        let p = path.join("test");
        scan_subfolder_join_set.spawn(thread_pool, move || scan_subfolder("cc", &p));
    }

    let mut fmt_join_set: JoinSet<()> = JoinSet::new();

    while let Some(paths) = scan_subfolder_join_set.join_next() {
        for path in paths {
            fmt_join_set.spawn(thread_pool, move || {
                format_file(clang_format, &path);
            });
        }
    }

    fmt_join_set.join_all();
}
