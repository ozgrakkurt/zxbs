use std::collections::HashSet;
use std::io::ErrorKind as IoErrorKind;
use std::sync::{Arc, Mutex};

use xxhash_rust::xxh3::xxh3_64 as hash_bytes;

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

    pub async fn run(&self) {
        let output = match tokio::process::Command::new(self.cxx)
            .args(self.cxxflags)
            .arg(self.input)
            .arg(format!("-fmodule-output={}", self.pcm_output))
            .arg("-c")
            .arg("-o")
            .arg(self.object_output)
            .output()
            .await
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

    pub async fn run(&self) {
        let output = match tokio::process::Command::new(self.cxx)
            .args(self.cxxflags)
            .arg(self.input)
            .arg("-c")
            .arg("-o")
            .arg(self.output)
            .output()
            .await
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

    pub async fn run(&self) {
        let output = match tokio::process::Command::new(self.ar)
            .arg("crs")
            .arg(self.output)
            .args(self.inputs.as_slice())
            .output()
            .await
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

    pub async fn run(&self) {
        let output = match tokio::process::Command::new(self.cxx)
            .args(self.ldflags)
            .arg("-o")
            .arg(self.output)
            .arg(self.object)
            .args(self.deps)
            .output()
            .await
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

    pub async fn run(&self) {
        let output = match tokio::process::Command::new(self.path)
            .args(self.args)
            .output()
            .await
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
                "Running a binary failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }
    }
}

pub struct Format {
    pub clang_format: &'static str,
    pub path: &'static str,
}

impl Format {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("FMT ");
        out.push_str(self.path);

        out
    }

    pub async fn run(&self) {
        let output = match tokio::process::Command::new(self.clang_format)
            .arg("-i")
            .arg("--style={BasedOnStyle: LLVM, IndentWidth: 4}")
            .arg(self.path)
            .output()
            .await
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
                "Running clang format failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }
    }
}

// Dummy command to be used when running tests.
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
        let mut out = String::with_capacity(1024);

        out.push_str("");

        out
    }

    pub async fn run(&self) {
        let mut finished = self.finished.lock().unwrap();

        for input in self.inputs.iter() {
            assert!(finished.contains(input));
        }

        for &output in self.outputs.iter() {
            assert!(finished.insert(output));
        }
    }
}

// ### HOW CACHING WORKS
// # before scan -> Load cache state.
// # scan -> do basic hash of individual files.
// # plan -> calculate aggregate hashes and check against db to
// emit either CacheLoad or regular build + CacheStore. Update cache use
// timestamps in state. Also save cached file size into the cache state.
// # after build -> Save cache state and cleanup old stuff if cache
// is too big.
//

// Load a file to output from cache.
// Checks the file in the cache is expected size and has the expected hash.
pub struct CacheLoad {
    pub cache_files_path: &'static str,
    pub output_path: &'static str,
    pub input_hash: u64,
    pub expected_hash: u64,
    pub expected_size: u32,
}

impl CacheLoad {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("CACHE_LOAD ");
        out.push_str(&format!("{:x}", self.input_hash));

        out
    }

    pub async fn run(&self) {
        let cache_path = format!("{}/{:x}", self.cache_files_path, self.input_hash);
        let data = tokio::fs::read(&cache_path).await.unwrap();
        assert!(u32::try_from(data.len()).unwrap() == self.expected_size);
        let hash = hash_bytes(&data);
        assert!(hash == self.expected_hash);
        tokio::fs::write(self.output_path, &data).await.unwrap();
    }
}

pub struct CacheStore {
    pub cache_files_path: &'static str,
    pub file_path: &'static str,
    pub input_hash: u64,
}

// CACHE STORES MIGHT BE INTERRUPTED BECAUSE OTHER TASKS FAIL WHILE
// CACHE STORE IS RUNNING.
//
// CacheStore SHOULD ALWAYS CHECK if the target file is already there and do nothing
// if it is.
//
// Doing this should cover (compilation etc.) failure resistence. Don't care about actual crash resistence.
//
// We need to do this since any failure causes program abort.
impl CacheStore {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("CACHE_STORE ");
        out.push_str(&format!("{:x}", self.input_hash));

        out
    }

    pub async fn run(&self) {
        let data = tokio::fs::read(self.file_path).await.unwrap();
        let expected_hash = hash_bytes(&data);

        let cache_path = format!("{}/{:x}", self.cache_files_path, self.input_hash);
        // check if the file we are intending to store is already there.
        // we need to do this instead of "just overwriting" so we don't mess up the file in case
        // of interruption.
        match tokio::fs::read(&cache_path).await {
            Ok(data) => {
                let hash = hash_bytes(&data);
                assert!(hash == expected_hash);
                return;
            }
            Err(e) => {
                if e.kind() != IoErrorKind::NotFound {
                    panic!(
                        "Unexpected err while checking cached file: {}: {:?}",
                        cache_path,
                        e.kind()
                    );
                }
            }
        };

        // at this point we know there is no file at the destination path so we can freely write what we want.
        // it is ok if our write gets interrupted because of program close because the existence of the file won't be
        // registered until the build finishes and we are sure there won't be any failures.

        tokio::fs::write(&cache_path, &data).await.unwrap();
    }
}

pub enum Command {
    CompileModule(CompileModule),
    CompileToObject(CompileToObject),
    ArchiveLib(ArchiveLib),
    LinkBinary(LinkBinary),
    RunBinary(RunBinary),
    CacheLoad(CacheLoad),
    CacheStore(CacheStore),
    Format(Format),
    TestCmd(TestCmd),
}

impl Command {
    pub fn print(&self) -> String {
        match self {
            Command::CompileModule(x) => x.print(),
            Command::CompileToObject(x) => x.print(),
            Command::ArchiveLib(x) => x.print(),
            Command::LinkBinary(x) => x.print(),
            Command::RunBinary(x) => x.print(),
            Command::CacheLoad(x) => x.print(),
            Command::CacheStore(x) => x.print(),
            Command::Format(x) => x.print(),
            Command::TestCmd(x) => x.print(),
        }
    }

    pub async fn run(&self) {
        match self {
            Command::CompileModule(x) => x.run().await,
            Command::CompileToObject(x) => x.run().await,
            Command::ArchiveLib(x) => x.run().await,
            Command::LinkBinary(x) => x.run().await,
            Command::RunBinary(x) => x.run().await,
            Command::CacheLoad(x) => x.run().await,
            Command::CacheStore(x) => x.run().await,
            Command::Format(x) => x.run().await,
            Command::TestCmd(x) => x.run().await,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Command::CompileModule(_) => "CompileModule",
            Command::CompileToObject(_) => "CompileToObject",
            Command::ArchiveLib(_) => "ArchiveLib",
            Command::LinkBinary(_) => "LinkBinary",
            Command::RunBinary(_) => "RunBinary",
            Command::CacheLoad(_) => "CacheLoad",
            Command::CacheStore(_) => "CacheStore",
            Command::Format(_) => "Format",
            Command::TestCmd(_) => "TestCmd",
        }
    }
}
