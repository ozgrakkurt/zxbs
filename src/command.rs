use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::cache::write_wal_file as write_cache_wal_file;
use crate::common::make_tmp_path;
use crate::hash::{Hash, hash_bytes};
use crate::interner::Str;

pub struct CompileModule {
    pub cxx: &'static str,
    pub cxxflags: &'static [&'static str],
    pub input: &'static str,
    pub object_output: &'static str,
    pub pcm_output: &'static str,
}

impl CompileModule {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<cxx> <cxxflags> ");

        out.push_str(self.input);

        out.push_str(" -fmodule-output=");

        out.push_str(self.pcm_output);

        out.push_str(" -c -o ");

        out.push_str(self.object_output);

        out
    }

    pub fn run(&self) {
        let tmp_object_output = make_tmp_path(self.object_output);
        let tmp_pcm_output = make_tmp_path(self.pcm_output);

        let output = match std::process::Command::new(self.cxx)
            .args(self.cxxflags)
            .arg(self.input)
            .arg(format!("-fmodule-output={}", tmp_pcm_output))
            .arg("-c")
            .arg("-o")
            .arg(&tmp_object_output)
            .output()
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Compile module (.cppm) Failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }

        std::fs::rename(&tmp_object_output, self.object_output).unwrap();
        std::fs::rename(&tmp_pcm_output, self.pcm_output).unwrap();
    }
}

pub struct CompileToObject {
    pub cxx: &'static str,
    pub cxxflags: &'static [&'static str],
    pub input: &'static str,
    pub output: &'static str,
}

impl CompileToObject {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<cxx> <cxxflags> ");

        out.push_str(self.input);

        out.push_str(" -c -o ");

        out.push_str(self.output);

        out
    }

    pub fn run(&self) {
        let tmp_output = make_tmp_path(self.output);

        let output = match std::process::Command::new(self.cxx)
            .args(self.cxxflags)
            .arg(self.input)
            .arg("-c")
            .arg("-o")
            .arg(&tmp_output)
            .output()
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Compile to .o Failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }

        std::fs::rename(&tmp_output, self.output).unwrap();
    }
}

pub struct ArchiveLib {
    pub ar: &'static str,
    pub output: &'static str,
    pub inputs: Vec<&'static str>,
}

impl ArchiveLib {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<ar> crs ");

        out.push_str(self.output);

        for input in self.inputs.iter() {
            out.push(' ');
            out.push_str(input);
        }

        out
    }

    pub fn run(&self) {
        let tmp_output = make_tmp_path(self.output);

        let output = match std::process::Command::new(self.ar)
            .arg("crs")
            .arg(&tmp_output)
            .args(self.inputs.as_slice())
            .output()
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Archiving a library failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }

        std::fs::rename(&tmp_output, self.output).unwrap();
    }
}

pub struct LinkBinary {
    pub cxx: &'static str,
    pub ldflags: &'static [&'static str],
    pub output: &'static str,
    pub object: &'static str,
    pub deps: &'static [&'static str],
}

impl LinkBinary {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<cxx> <ldflags> -o ");

        out.push_str(self.output);

        out.push(' ');
        out.push_str(self.object);

        for dep in self.deps.iter() {
            out.push(' ');
            out.push_str(dep);
        }

        out
    }

    pub fn run(&self) {
        let tmp_output = make_tmp_path(self.output);
        let output = match std::process::Command::new(self.cxx)
            .args(self.ldflags)
            .arg("-o")
            .arg(&tmp_output)
            .arg(self.object)
            .args(self.deps)
            .output()
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Linking a binary failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }

        std::fs::rename(&tmp_output, self.output).unwrap();
    }
}

pub struct RunTest {
    pub path: &'static str,
    pub stamp_hash: Hash,
    pub stamp_path: &'static str,
}

impl RunTest {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("RUN_TEST ");
        out.push_str(self.path);

        out
    }

    pub fn run(&self) {
        let output = match std::process::Command::new(self.path).output() {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Running a test failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }

        let tmp_output = make_tmp_path(self.stamp_path);
        std::fs::write(&tmp_output, self.stamp_hash.to_le_bytes()).unwrap();
        std::fs::rename(&tmp_output, self.stamp_path).unwrap()
    }
}

pub struct RunBinary {
    pub path: &'static str,
    pub args: &'static [&'static str],
}

impl RunBinary {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("RUN_BINARY ");
        out.push_str(self.path);

        out
    }

    pub fn run(&self) {
        let output = match std::process::Command::new(self.path)
            .stderr(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stdin(std::process::Stdio::inherit())
            .args(self.args)
            .output()
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            eprintln!(
                "Running a binary failed. Command:\n\t{}\nStatus: {}",
                self.print(),
                output.status,
            );
            std::process::exit(output.status.code().unwrap_or(1));
        }
    }
}

// Dummy command to be used when running tests and fuzzing.
//
// Just checks that everything is consistent and the build isn't running
// in the wrong order.
pub struct TestCmd {
    pub finished: Arc<Mutex<HashSet<Str>>>,
    pub inputs: Vec<Str>,
    pub outputs: Vec<Str>,
}

impl TestCmd {
    pub fn print(&self) -> String {
        String::from("Test Command")
    }

    pub fn run(&self) {
        let mut finished = self.finished.lock().unwrap();

        for input in self.inputs.iter() {
            assert!(finished.contains(input));
        }

        for &output in self.outputs.iter() {
            assert!(finished.insert(output));
        }
    }
}

// Load a file to output from cache.
// Checks the file in the cache has the expected hash.
pub struct CacheLoad {
    pub cache_files_path: &'static str,
    pub output_path: &'static str,
    pub deep_hash: Hash,
    pub expected_hash: Hash,
}

impl CacheLoad {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("CACHE_LOAD ");
        out.push_str(&format!("{:x}", self.deep_hash));

        out
    }

    pub fn run(&self) {
        let cache_path = format!("{}/{:x}", self.cache_files_path, self.deep_hash);
        let data = std::fs::read(&cache_path).unwrap();
        let hash = hash_bytes(&data);
        assert!(hash == self.expected_hash);

        // it is crucial to use copy so the file keeps same permissions.
        // we need it to be executable if it is an executable file.
        let tmp_path = make_tmp_path(self.output_path);
        std::fs::copy(&cache_path, &tmp_path).unwrap();
        std::fs::rename(&tmp_path, self.output_path).unwrap();
    }
}

pub struct CacheStore {
    pub cache_files_path: &'static str,
    pub file_path: &'static str,
    pub deep_hash: Hash,
}

impl CacheStore {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("CACHE_STORE ");
        out.push_str(&format!("{:x}", self.deep_hash));

        out
    }

    pub fn run(&self) {
        let data = std::fs::read(self.file_path).unwrap();
        let output_hash = hash_bytes(&data);

        let cache_path = format!("{}/{:x}", self.cache_files_path, self.deep_hash);

        // it is crucial to use copy so the file keeps same permissions.
        // we need it to be executable if it is an executable file.
        let tmp_path = make_tmp_path(&cache_path);
        std::fs::copy(self.file_path, &tmp_path).unwrap();
        std::fs::rename(&tmp_path, &cache_path).unwrap();

        let output_size: u32 = u32::try_from(data.len()).unwrap();

        write_cache_wal_file(
            Path::new(self.cache_files_path),
            self.deep_hash,
            output_hash,
            output_size,
        );
    }
}

pub enum Command {
    CompileModule(CompileModule),
    CompileToObject(CompileToObject),
    ArchiveLib(ArchiveLib),
    LinkBinary(LinkBinary),
    RunBinary(RunBinary),
    RunTest(RunTest),
    CacheLoad(CacheLoad),
    CacheStore(CacheStore),
    TestCmd(TestCmd),
}

#[derive(Clone, Copy, Debug)]
pub enum CommandKind {
    CompileModule,
    CompileToObject,
    ArchiveLib,
    LinkBinary,
    RunBinary,
    RunTest,
    CacheLoad,
    CacheStore,
    TestCmd,
}

impl Command {
    pub fn print(&self) -> String {
        match self {
            Command::CompileModule(x) => x.print(),
            Command::CompileToObject(x) => x.print(),
            Command::ArchiveLib(x) => x.print(),
            Command::LinkBinary(x) => x.print(),
            Command::RunBinary(x) => x.print(),
            Command::RunTest(x) => x.print(),
            Command::CacheLoad(x) => x.print(),
            Command::CacheStore(x) => x.print(),
            Command::TestCmd(x) => x.print(),
        }
    }

    pub fn run(&self) {
        match self {
            Command::CompileModule(x) => x.run(),
            Command::CompileToObject(x) => x.run(),
            Command::ArchiveLib(x) => x.run(),
            Command::LinkBinary(x) => x.run(),
            Command::RunBinary(x) => x.run(),
            Command::RunTest(x) => x.run(),
            Command::CacheLoad(x) => x.run(),
            Command::CacheStore(x) => x.run(),
            Command::TestCmd(x) => x.run(),
        }
    }

    pub fn kind(&self) -> CommandKind {
        match self {
            Command::CompileModule(_) => CommandKind::CompileModule,
            Command::CompileToObject(_) => CommandKind::CompileToObject,
            Command::ArchiveLib(_) => CommandKind::ArchiveLib,
            Command::LinkBinary(_) => CommandKind::LinkBinary,
            Command::RunBinary(_) => CommandKind::RunBinary,
            Command::RunTest(_) => CommandKind::RunTest,
            Command::CacheLoad(_) => CommandKind::CacheLoad,
            Command::CacheStore(_) => CommandKind::CacheStore,
            Command::TestCmd(_) => CommandKind::TestCmd,
        }
    }
}
